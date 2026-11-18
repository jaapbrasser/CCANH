//! Task API visibility and cancellation, the `tasks` WebSocket topic, and
//! runtime settings.

mod common;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::config::{Config, Mode};
use crane::events::Event;
use crane::tasks::Creator;
use serde_json::{json, Value};

fn demo() -> Config {
    Config {
        mode: Mode::Demo,
        ..Config::default()
    }
}

#[tokio::test]
async fn test_connection_task_reports_progress_and_result() {
    let app = TestApp::with_config(demo()).await;
    let (admin, cookie) = app.signed_in("admin", Role::Admin).await;
    let mut events = app.state.events.subscribe();
    let res = app
        .send(
            req(Method::POST, "/api/v1/cameras/test-connection")
                .cookie(&cookie)
                .json(json!({ "host": "192.0.2.11" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED);
    let task = res.json();
    assert_eq!(task["kind"], "test_connection");
    assert_eq!(task["status"], "queued");
    assert_eq!(task["created_by"], admin.id.to_string());
    let id = task["id"].as_str().unwrap().to_owned();

    let done = app.wait_task(&cookie, &id).await;
    assert_eq!(done["status"], "succeeded");
    assert_eq!(done["progress_current"], 4);
    assert_eq!(done["progress_total"], 4);
    assert_eq!(done["result"]["ok"], true);

    let mut messages = Vec::new();
    while let Ok(Event::TaskUpdated { task }) = events.try_recv() {
        if let Some(m) = task.message {
            messages.push(m);
        }
    }
    assert!(
        messages.iter().any(|m| m == "Verifying credentials"),
        "{messages:?}"
    );
}

#[tokio::test]
async fn viewers_only_see_and_cancel_their_own_tasks() {
    let app = TestApp::new().await;
    let (_, admin_cookie) = app.signed_in("admin", Role::Admin).await;
    let (alice, alice_cookie) = app.signed_in("alice", Role::Viewer).await;
    let (_, bob_cookie) = app.signed_in("bob", Role::Viewer).await;

    let slow = |user: Creator| {
        app.state
            .tasks
            .spawn("demo", None, Some(user), |ctx| async move {
                ctx.cancel.cancelled().await;
                Ok(Value::Null)
            })
    };
    let task = slow(Creator {
        id: alice.id,
        username: "alice".into(),
    })
    .await
    .unwrap();
    let id = task.id.to_string();

    // Alice sees it; Bob gets 404 for get and cancel and an empty list.
    let get =
        |cookie: &str| app.send(req(Method::GET, &format!("/api/v1/tasks/{id}")).cookie(cookie));
    assert_eq!(get(&alice_cookie).await.status, StatusCode::OK);
    assert_eq!(get(&bob_cookie).await.status, StatusCode::NOT_FOUND);
    assert_eq!(get(&admin_cookie).await.status, StatusCode::OK);
    let bobs = app
        .send(req(Method::GET, "/api/v1/tasks").cookie(&bob_cookie))
        .await;
    assert_eq!(bobs.json()["total"], 0);
    let all = app
        .send(req(Method::GET, "/api/v1/tasks").cookie(&admin_cookie))
        .await;
    assert_eq!(all.json()["total"], 1);

    let cancel = |cookie: &str| {
        app.send(req(Method::POST, &format!("/api/v1/tasks/{id}/cancel")).cookie(cookie))
    };
    assert_eq!(cancel(&bob_cookie).await.status, StatusCode::NOT_FOUND);
    assert_eq!(cancel(&alice_cookie).await.status, StatusCode::ACCEPTED);
    let done = app.wait_task(&alice_cookie, &id).await;
    assert_eq!(done["status"], "cancelled");
    assert_eq!(cancel(&alice_cookie).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn settings_are_validated_audited_and_applied() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;

    let view = app
        .send(req(Method::GET, "/api/v1/settings").cookie(&admin))
        .await;
    assert_eq!(view.status, StatusCode::OK);
    assert_eq!(view.json()["current"]["monitoring"]["interval_secs"], 10);
    assert_eq!(view.json()["defaults"]["monitoring"]["interval_secs"], 10);
    assert!(view.json()["monitor"]["checks_total"].is_number());

    let bad = app
        .send(
            req(Method::PATCH, "/api/v1/settings/monitoring")
                .cookie(&admin)
                .json(json!({ "interval_secs": 1 })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.json()["detail"]
        .as_str()
        .unwrap()
        .contains("interval_secs"));

    let rx = app.state.settings.subscribe();
    let ok = app
        .send(
            req(Method::PATCH, "/api/v1/settings/monitoring")
                .cookie(&admin)
                .json(json!({ "interval_secs": 30, "offline_max_interval_secs": 300 })),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.json()["interval_secs"], 30);
    assert!(
        rx.has_changed().unwrap(),
        "change is published to running services"
    );
    assert_eq!(app.state.settings.current().monitoring.interval_secs, 30);

    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=settings.update").cookie(&admin))
        .await;
    let entry = &audit.json()["items"][0];
    assert_eq!(entry["details"]["from"]["interval_secs"], 10);
    assert_eq!(entry["details"]["to"]["interval_secs"], 30);

    let reset = app
        .send(req(Method::DELETE, "/api/v1/settings/monitoring").cookie(&admin))
        .await;
    assert_eq!(reset.json()["interval_secs"], 10);
}

#[tokio::test]
async fn changed_interval_reaches_the_running_monitor() {
    // With a 60 s interval a camera is checked once at start-up; dropping the
    // interval to 2 s at runtime makes it checked again within a few seconds.
    let mut config = demo();
    config.monitoring.interval_secs = 60;
    config.monitoring.offline_max_interval_secs = 60;
    let app = TestApp::with_config(config).await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    app.send(
        req(Method::POST, "/api/v1/cameras")
            .cookie(&admin)
            .json(json!({
                "name": "Hook", "host": "192.0.2.11"
            })),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crane::serve(
        listener,
        app.state.clone(),
        None,
        shutdown.clone(),
    ));

    let checks = || app.state.monitor_stats.snapshot().checks_total;
    for _ in 0..100 {
        if checks() >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(checks(), 1);

    app.send(
        req(Method::PATCH, "/api/v1/settings/monitoring")
            .cookie(&admin)
            .json(json!({ "interval_secs": 2, "jitter_percent": 0 })),
    )
    .await;
    // The pending check was scheduled 60 s out; a shorter interval pulls it in.
    for _ in 0..200 {
        if checks() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    shutdown.cancel();
    server.await.unwrap().unwrap();
    assert!(checks() >= 2, "monitor did not pick up the new interval");
}
