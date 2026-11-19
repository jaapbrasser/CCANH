//! Discovery in demo mode (SPEC §5; Phase 8 acceptance: scanning a /24 never
//! blocks the API, results stream live, cancel works, duplicates are flagged).

mod common;

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::config::{Config, Mode};
use crane::events::Event;
use serde_json::{json, Value};

async fn demo_app() -> (TestApp, String) {
    let app = TestApp::with_config(Config {
        mode: Mode::Demo,
        ..Config::default()
    })
    .await;
    // The demo fleet is registered, so scanning its range finds duplicates.
    crane::demo::seed(&app.state.db, &*app.state.credentials)
        .await
        .unwrap();
    let (_, admin) = app.signed_in("scanner", Role::Admin).await;
    (app, admin)
}

async fn start(app: &TestApp, admin: &str, body: Value) -> common::Response {
    app.send(
        req(Method::POST, "/api/v1/discovery/jobs")
            .cookie(admin)
            .json(body),
    )
    .await
}

async fn job(app: &TestApp, admin: &str, id: &str) -> Value {
    app.send(req(Method::GET, &format!("/api/v1/discovery/jobs/{id}")).cookie(admin))
        .await
        .json()
}

#[tokio::test]
async fn scanning_a_24_streams_results_and_flags_duplicates() {
    let (app, admin) = demo_app().await;
    let mut events = app.state.events.subscribe();

    let started = start(&app, &admin, json!({ "ranges": ["192.0.2.0/24"] })).await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.text());
    let id = started.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(started.json()["address_count"], 254);

    // The API stays responsive while the scan runs.
    let t = Instant::now();
    let health = app.send(req(Method::GET, "/api/v1/health")).await;
    assert_eq!(health.status, StatusCode::OK);
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "{:?}",
        t.elapsed()
    );

    // Results arrive as they're found, before the scan finishes.
    let mut found = 0;
    let mut progress = 0;
    let mut first_found_while_running = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(Event::DiscoveryFound { job_id, .. })) if job_id.to_string() == id => {
                if found == 0 {
                    let task = app
                        .send(req(Method::GET, &format!("/api/v1/tasks/{id}")).cookie(&admin))
                        .await
                        .json();
                    first_found_while_running = task["status"] == "running";
                }
                found += 1;
            }
            Ok(Ok(Event::DiscoveryProgress {
                job_id,
                scanned,
                total,
                ..
            })) if job_id.to_string() == id => {
                progress += 1;
                if scanned == total {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }
    assert!(
        first_found_while_running,
        "results stream before the scan ends"
    );
    assert!(progress >= 2, "progress is reported along the way");

    let task = app.wait_task(&admin, &id).await;
    assert_eq!(task["status"], "succeeded", "{task}");
    let detail = job(&app, &admin, &id).await;
    assert_eq!(detail["scanned"], 254);
    // 11 online fleet cameras (the Rail Spur one is offline) + 6 unregistered
    // demo devices; the router at .1 answers but isn't a camera.
    assert_eq!(detail["found"], 17);
    assert_eq!(detail["responsive"], 18);
    assert_eq!(found, 17);
    let results = detail["results"].as_array().unwrap();
    let dup = |ip: &str| results.iter().find(|r| r["ip"] == ip).unwrap()["duplicate_of"].clone();
    assert_eq!(dup("192.0.2.11")["reason"], "ip");
    assert_eq!(dup("192.0.2.11")["camera_name"], "A1 Crane 1 – Hook");
    assert_eq!(dup("192.0.2.41"), Value::Null, "new device");
    assert_eq!(
        results
            .iter()
            .filter(|r| r["duplicate_of"].is_null())
            .count(),
        6
    );

    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=discovery.start").cookie(&admin))
        .await;
    assert_eq!(audit.json()["total"], 1);
}

#[tokio::test]
async fn caps_need_confirmation_and_cancel_stops_the_scan() {
    let (app, admin) = demo_app().await;
    let preview = app
        .send(
            req(Method::POST, "/api/v1/discovery/preview")
                .cookie(&admin)
                .json(json!({ "ranges": ["10.0.0.0/18"] })),
        )
        .await
        .json();
    assert_eq!(preview["address_count"], 16382);
    assert_eq!(preview["needs_confirmation"], true);

    let refused = start(&app, &admin, json!({ "ranges": ["10.0.0.0/18"] })).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.text().contains("Confirm"), "{}", refused.text());
    let huge = start(
        &app,
        &admin,
        json!({ "ranges": ["10.0.0.0/15"], "confirm_large": true }),
    )
    .await;
    assert_eq!(
        huge.status,
        StatusCode::BAD_REQUEST,
        "past the hard ceiling"
    );
    let bad = start(&app, &admin, json!({ "ranges": ["cameras.example"] })).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    let started = start(
        &app,
        &admin,
        json!({ "ranges": ["10.0.0.0/18"], "confirm_large": true }),
    )
    .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.text());
    let id = started.json()["id"].as_str().unwrap().to_owned();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let cancel = app
        .send(req(Method::POST, &format!("/api/v1/tasks/{id}/cancel")).cookie(&admin))
        .await;
    assert!(cancel.status.is_success(), "{}", cancel.text());
    let task = app.wait_task(&admin, &id).await;
    assert_eq!(task["status"], "cancelled");
    let detail = job(&app, &admin, &id).await;
    let scanned = detail["scanned"].as_u64().unwrap();
    assert!(scanned > 0 && scanned < 16382, "scanned {scanned}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        job(&app, &admin, &id).await["scanned"],
        scanned,
        "nothing runs after cancel"
    );
}

#[tokio::test]
async fn registering_approved_results() {
    let (app, admin) = demo_app().await;
    let tag = app
        .send(
            req(Method::POST, "/api/v1/tags")
                .cookie(&admin)
                .json(json!({ "name": "Discovered" })),
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = start(
        &app,
        &admin,
        json!({ "ranges": ["192.0.2.10-12", "192.0.2.41-42"] }),
    )
    .await;
    let id = started.json()["id"].as_str().unwrap().to_owned();
    app.wait_task(&admin, &id).await;
    let results = job(&app, &admin, &id).await["results"]
        .as_array()
        .unwrap()
        .clone();
    let by_ip = |ip: &str| {
        results.iter().find(|r| r["ip"] == ip).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (new1, new2, dup) = (
        by_ip("192.0.2.41"),
        by_ip("192.0.2.42"),
        by_ip("192.0.2.11"),
    );

    let r = app
        .send(
            req(Method::POST, &format!("/api/v1/discovery/jobs/{id}/register"))
                .cookie(&admin)
                .json(json!({ "result_ids": [new1, new2, dup], "tag_ids": [tag], "monitoring_enabled": false })),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let out = r.json();
    assert_eq!(out["registered"].as_array().unwrap().len(), 2);
    assert_eq!(
        out["registered"][0]["camera"]["tags"][0]["name"],
        "Discovered"
    );
    assert_eq!(out["registered"][0]["camera"]["host"], "192.0.2.41");
    // .41 answered on HTTPS (simulated devices show no certificate to pin, so
    // the default certificate check applies), .42 only on HTTP: it gets the
    // camera default for Basic over HTTP (on, ADR 0011) and no certificate mode.
    let detail = |cam: &Value| {
        let id = cam["camera"]["id"].as_str().unwrap().to_owned();
        let app = &app;
        let admin = &admin;
        async move {
            app.send(req(Method::GET, &format!("/api/v1/cameras/{id}")).cookie(admin))
                .await
                .json()
        }
    };
    let c41 = detail(&out["registered"][0]).await;
    assert_eq!(c41["tls_mode"], "insecure_skip_verify");
    assert_eq!(c41["allow_basic_over_http"], false);
    let c42 = detail(&out["registered"][1]).await;
    assert_eq!(c42["scheme"], "http");
    assert_eq!(c42["tls_mode"], Value::Null);
    assert_eq!(c42["allow_basic_over_http"], true);
    assert_eq!(out["skipped"][0]["result_id"], dup.as_str());
    assert!(out["skipped"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("A1 Crane 1 – Hook"));

    // Registered results now show as such, and can't be registered twice.
    let after = job(&app, &admin, &id).await["results"]
        .as_array()
        .unwrap()
        .clone();
    let r41 = after.iter().find(|r| r["ip"] == "192.0.2.41").unwrap();
    assert!(r41["registered_camera_id"].is_string());
    assert_eq!(r41["duplicate_of"], Value::Null);
    let again = app
        .send(
            req(
                Method::POST,
                &format!("/api/v1/discovery/jobs/{id}/register"),
            )
            .cookie(&admin)
            .json(json!({ "result_ids": [new1] })),
        )
        .await
        .json();
    assert_eq!(again["registered"], json!([]));

    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.create").cookie(&admin))
        .await;
    assert_eq!(audit.json()["total"], 2);

    let (_, viewer) = app.signed_in("viewer2", Role::Viewer).await;
    let denied = app
        .send(req(Method::GET, &format!("/api/v1/discovery/jobs/{id}")).cookie(&viewer))
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn testing_credentials_before_registering() {
    let (app, admin) = demo_app().await;
    let started = start(&app, &admin, json!({ "ranges": ["192.0.2.41-42"] })).await;
    let id = started.json()["id"].as_str().unwrap().to_owned();
    app.wait_task(&admin, &id).await;
    let ids: Vec<String> = job(&app, &admin, &id).await["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 2);

    let test = |password: &'static str| {
        let body = json!({
            "result_ids": ids,
            "credentials": { "type": "inline", "username": "root", "password": password },
        });
        let (app, admin, id) = (&app, &admin, &id);
        async move {
            let r = app
                .send(
                    req(
                        Method::POST,
                        &format!("/api/v1/discovery/jobs/{id}/test-credentials"),
                    )
                    .cookie(admin)
                    .json(body),
                )
                .await;
            assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
            assert!(!r.text().contains(password), "secrets are never echoed");
            let task_id = r.json()["id"].as_str().unwrap().to_owned();
            app.wait_task(admin, &task_id).await
        }
    };

    let good = test("right-password").await;
    assert_eq!(good["status"], "succeeded", "{good}");
    let results = good["result"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r["ok"] == true), "{good}");

    // Demo cameras reject the password "wrong" (docs/demo.md).
    let bad = test("wrong").await;
    let results = bad["result"]["results"].as_array().unwrap();
    assert!(results
        .iter()
        .all(|r| r["ok"] == false && r["error_category"] == "auth_failed"));

    let too_many = app
        .send(
            req(
                Method::POST,
                &format!("/api/v1/discovery/jobs/{id}/test-credentials"),
            )
            .cookie(&admin)
            .json(json!({ "result_ids": [], "credentials": { "type": "none" } })),
        )
        .await;
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
    // Nothing was registered.
    let cams = app
        .send(req(Method::GET, "/api/v1/cameras?q=192.0.2.4").cookie(&admin))
        .await;
    assert_eq!(cams.json()["total"], 0);
}
