//! Camera registry API, status monitoring end to end, and demo mode.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::cameras::{self, CameraState};
use crane::config::{Config, Mode};
use crane::events::Event;
use crane::providers::ErrorCategory;
use serde_json::json;

fn demo_config() -> Config {
    Config {
        mode: Mode::Demo,
        ..Config::default()
    }
}

#[tokio::test]
async fn admin_registers_and_manages_cameras() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let mut events = app.state.events.subscribe();

    let created = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Crane 1", "host": "192.0.2.50"
                })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let body = created.json();
    let id = body["id"].as_str().unwrap().to_owned();
    // Omitted fields come from the camera defaults (HTTP, Basic allowed: ADR 0011).
    assert_eq!(body["port"], 80);
    assert_eq!(body["scheme"], "http");
    assert_eq!(body["tls_mode"], serde_json::Value::Null);
    assert_eq!(body["allow_basic_over_http"], true);
    assert_eq!(body["name_pending"], false);
    assert_eq!(body["status"]["state"], "unknown");
    assert!(matches!(events.try_recv(), Ok(Event::CameraChanged { .. })));

    let dup = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Again", "host": "192.0.2.50"
                })),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);

    // The name is optional: the address stands in until the camera names itself.
    let unnamed = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({ "host": "192.0.2.51", "monitoring_enabled": false })),
        )
        .await;
    assert_eq!(unnamed.status, StatusCode::CREATED, "{}", unnamed.text());
    assert_eq!(unnamed.json()["name"], "192.0.2.51");
    assert_eq!(unnamed.json()["name_pending"], true);
    let unnamed_id = unnamed.json()["id"].as_str().unwrap().to_owned();
    app.send(req(Method::DELETE, &format!("/api/v1/cameras/{unnamed_id}")).cookie(&admin))
        .await;

    let bad = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Bad", "host": "http://admin:pw@192.0.2.51/"
                })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(!bad.text().contains("pw@"), "input must not be echoed back");

    let pinned = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Pinned", "host": "cam.local", "scheme": "https",
                    "tls_mode": "pinned"
                })),
        )
        .await;
    assert_eq!(
        pinned.status,
        StatusCode::BAD_REQUEST,
        "pinned needs a fingerprint"
    );

    let patched = app
        .send(
            req(Method::PATCH, &format!("/api/v1/cameras/{id}"))
                .cookie(&admin)
                .json(json!({ "name": "Crane 1 – Hook", "monitoring_enabled": false })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.json()["name"], "Crane 1 – Hook");
    assert_eq!(patched.json()["monitoring_enabled"], false);

    let deleted = app
        .send(req(Method::DELETE, &format!("/api/v1/cameras/{id}")).cookie(&admin))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = app
        .send(req(Method::GET, &format!("/api/v1/cameras/{id}")).cookie(&admin))
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.").cookie(&admin))
        .await;
    let actions: Vec<_> = audit.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        actions,
        [
            "camera.delete",
            "camera.update",
            "camera.delete",
            "camera.create",
            "camera.create"
        ]
    );
}

#[tokio::test]
async fn viewers_can_read_cameras() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    for (name, host) in [("Bravo", "192.0.2.61"), ("Alpha", "192.0.2.60")] {
        app.send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": name, "host": host
                })),
        )
        .await;
    }
    let list = app
        .send(req(Method::GET, "/api/v1/cameras?sort=name&order=desc").cookie(&viewer))
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    assert_eq!(list.json()["total"], 2);
    assert_eq!(list.json()["items"][0]["name"], "Bravo");

    let search = app
        .send(req(Method::GET, "/api/v1/cameras?q=192.0.2.60").cookie(&viewer))
        .await;
    assert_eq!(search.json()["items"][0]["name"], "Alpha");

    let online = app
        .send(req(Method::GET, "/api/v1/cameras?status=online").cookie(&viewer))
        .await;
    assert_eq!(online.json()["total"], 0);
}

#[tokio::test]
async fn refresh_fetches_identity_in_demo_mode() {
    let app = TestApp::with_config(demo_config()).await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let id = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Hook cam", "host": "192.0.2.11"
                })),
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let res = app
        .send(req(Method::POST, &format!("/api/v1/cameras/{id}/refresh")).cookie(&admin))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text());
    let cam = res.json();
    assert_eq!(cam["manufacturer"], "AXIS");
    assert_eq!(cam["model"], "AXIS Q6135-LE");
    let caps = cam["capabilities"].as_array().unwrap();
    assert!(caps
        .iter()
        .any(|c| c["key"] == "ptz.control" && c["supported"] == true));
    assert!(caps.iter().any(|c| c["supported"] == false));
    assert!(!cam["stream_profiles"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn refresh_reports_camera_errors_with_category() {
    let app = TestApp::with_config(demo_config()).await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let id = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Offline", "host": "192.0.2.34"
                })),
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let res = app
        .send(req(Method::POST, &format!("/api/v1/cameras/{id}/refresh")).cookie(&admin))
        .await;
    assert_eq!(res.status, StatusCode::BAD_GATEWAY);
    assert_eq!(res.json()["code"], "camera_error");
    assert_eq!(res.json()["error_category"], "unreachable");
}

#[tokio::test]
async fn status_history_requires_existing_camera() {
    let app = TestApp::new().await;
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    let res = app
        .send(
            req(
                Method::GET,
                &format!("/api/v1/cameras/{}/status-history", uuid::Uuid::now_v7()),
            )
            .cookie(&viewer),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_lists_demo_accounts_only_in_demo_mode() {
    let prod = TestApp::new().await;
    let res = prod.send(req(Method::GET, "/api/v1/health")).await;
    assert!(res.json().get("demo_accounts").is_none());

    let demo = TestApp::with_config(demo_config()).await;
    let res = demo.send(req(Method::GET, "/api/v1/health")).await;
    assert_eq!(res.json()["mode"], "demo");
    assert_eq!(res.json()["demo_accounts"][0]["username"], "admin");
}

/// Wait until `pred` holds for the camera, polling the database.
async fn wait_for<F>(app: &TestApp, id: uuid::Uuid, what: &str, pred: F) -> cameras::CameraDetail
where
    F: Fn(&cameras::CameraDetail) -> bool,
{
    for _ in 0..200 {
        if let Some(cam) = cameras::get(&app.state.db, id).await.unwrap() {
            if pred(&cam) {
                return cam;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn monitor_tracks_demo_fleet_end_to_end() {
    let mut config = demo_config();
    config.monitoring.interval_secs = 1;
    config.monitoring.failure_threshold = 2;
    let app = TestApp::with_config(config).await;
    crane::demo::seed(&app.state.db, &*app.state.credentials)
        .await
        .unwrap();
    let (cams, _) = cameras::list(
        &app.state.db,
        cameras::ListParams {
            search: None,
            state: None,
            sort: cameras::CameraSort::Name,
            descending: false,
            limit: 100,
            offset: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let by_host = |h: &str| cams.iter().find(|c| c.host == h).unwrap().id;
    let healthy = by_host("192.0.2.11");
    let offline = by_host("192.0.2.34");
    let auth = by_host("198.51.100.24");

    let mut events = app.state.events.subscribe();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crane::serve(
        listener,
        app.state.clone(),
        None,
        shutdown.clone(),
    ));

    let cam = wait_for(&app, healthy, "healthy camera online with identity", |c| {
        c.summary.status.state == CameraState::Online && c.identity_updated_at.is_some()
    })
    .await;
    assert_eq!(cam.summary.model.as_deref(), Some("AXIS Q6135-LE"));
    assert!(cam.summary.status.response_time_ms.is_some());

    let cam = wait_for(&app, offline, "offline camera", |c| {
        c.summary.status.state == CameraState::Offline
    })
    .await;
    assert_eq!(
        cam.summary.status.error_category,
        Some(ErrorCategory::Unreachable)
    );

    // Wrong credentials: the camera is reachable (anonymous status calls work),
    // and the credential problem is reported separately.
    let cam = wait_for(&app, auth, "auth-failure camera", |c| {
        c.summary.status.identity_error_category.is_some()
    })
    .await;
    assert_eq!(cam.summary.status.state, CameraState::Online);
    assert_eq!(
        cam.summary.status.identity_error_category,
        Some(ErrorCategory::AuthFailed)
    );
    assert!(cam.summary.status.error_category.is_none());

    // Let a few more checks run, then confirm history has one transition, not
    // one row per poll.
    wait_for(&app, healthy, "more checks", |c| {
        c.summary
            .status
            .last_checked_at
            .zip(c.summary.status.last_change_at)
            .is_some_and(|(checked, changed)| checked - changed > time::Duration::seconds(2))
    })
    .await;
    let (history, total) = cameras::history(&app.state.db, healthy, 10, 0)
        .await
        .unwrap();
    assert_eq!(total, 1, "{history:?}");
    assert_eq!(history[0].to_state, CameraState::Online);

    // Status changes were published on the event bus.
    let mut saw_status = false;
    while let Ok(event) = events.try_recv() {
        if matches!(event, Event::CameraStatus { camera_id, .. } if camera_id == healthy) {
            saw_status = true;
        }
    }
    assert!(saw_status);

    // The monitor is visible and healthy.
    let health = app.send(req(Method::GET, "/api/v1/health")).await;
    assert_eq!(health.json()["services"]["status_monitor"]["running"], true);

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn disabled_monitoring_is_not_checked() {
    let mut config = demo_config();
    config.monitoring.interval_secs = 1;
    let app = TestApp::with_config(config).await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let id: uuid::Uuid = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Quiet", "host": "192.0.2.12", "monitoring_enabled": false
                })),
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crane::serve(
        listener,
        app.state.clone(),
        None,
        shutdown.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let cam = cameras::get(&app.state.db, id).await.unwrap().unwrap();
    assert!(cam.summary.status.last_checked_at.is_none());
    shutdown.cancel();
    server.await.unwrap().unwrap();
}
