//! Per-user dashboards (SPEC §8): privacy, defaults, validation, persistence.

mod common;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use serde_json::{json, Value};

async fn camera(app: &TestApp, admin: &str, n: u8) -> String {
    let r = app
        .send(req(Method::POST, "/api/v1/cameras").cookie(admin).json(json!({
            "name": format!("Camera {n}"), "host": format!("192.0.2.{n}"), "monitoring_enabled": false
        })))
        .await;
    r.json()["id"].as_str().unwrap().to_owned()
}

async fn create(app: &TestApp, cookie: &str, body: Value) -> common::Response {
    app.send(
        req(Method::POST, "/api/v1/dashboards")
            .cookie(cookie)
            .json(body),
    )
    .await
}

async fn patch(app: &TestApp, cookie: &str, id: &str, body: Value) -> common::Response {
    app.send(
        req(Method::PATCH, &format!("/api/v1/dashboards/{id}"))
            .cookie(cookie)
            .json(body),
    )
    .await
}

#[tokio::test]
async fn defaults_follow_the_user_and_layouts_persist() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (a, b) = (camera(&app, &admin, 1).await, camera(&app, &admin, 2).await);

    let first = create(&app, &admin, json!({ "name": "Main" })).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.text());
    let first = first.json();
    assert_eq!(
        first["is_default"], true,
        "the first dashboard is the default"
    );
    assert_eq!(
        (first["rows"].as_u64(), first["cols"].as_u64()),
        (Some(2), Some(2))
    );
    let second = create(
        &app,
        &admin,
        json!({ "name": "Yard", "rows": 3, "cols": 4, "tiles": [{ "position": 11, "camera_id": a }] }),
    )
    .await
    .json();
    assert_eq!(second["is_default"], false);
    let (first_id, second_id) = (
        first["id"].as_str().unwrap(),
        second["id"].as_str().unwrap(),
    );

    // Rearranging persists; the default can move.
    let moved = patch(
        &app,
        &admin,
        second_id,
        json!({ "tiles": [{ "position": 0, "camera_id": b }, { "position": 5, "camera_id": a }], "make_default": true }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text());
    let list = app
        .send(req(Method::GET, "/api/v1/dashboards").cookie(&admin))
        .await
        .json();
    assert_eq!(list[0]["is_default"], false);
    assert_eq!(list[1]["is_default"], true);
    assert_eq!(
        list[1]["tiles"],
        json!([{ "position": 0, "camera_id": b }, { "position": 5, "camera_id": a }])
    );

    // Shrinking without new tiles drops what no longer fits.
    let shrunk = patch(&app, &admin, second_id, json!({ "rows": 1, "cols": 2 }))
        .await
        .json();
    assert_eq!(shrunk["tiles"], json!([{ "position": 0, "camera_id": b }]));

    // A deleted camera disappears from tiles.
    app.send(req(Method::DELETE, &format!("/api/v1/cameras/{b}")).cookie(&admin))
        .await;
    let got = app
        .send(req(Method::GET, &format!("/api/v1/dashboards/{second_id}")).cookie(&admin))
        .await
        .json();
    assert_eq!(got["tiles"], json!([]));

    // Deleting the default promotes the oldest remaining dashboard.
    let del = app
        .send(req(Method::DELETE, &format!("/api/v1/dashboards/{second_id}")).cookie(&admin))
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let list = app
        .send(req(Method::GET, "/api/v1/dashboards").cookie(&admin))
        .await
        .json();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], first_id);
    assert_eq!(list[0]["is_default"], true);
}

#[tokio::test]
async fn dashboards_are_private() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    let mine = create(&app, &viewer, json!({ "name": "Mine" })).await;
    assert_eq!(
        mine.status,
        StatusCode::CREATED,
        "viewers manage their own dashboards"
    );
    let id = mine.json()["id"].as_str().unwrap().to_owned();

    let theirs = app
        .send(req(Method::GET, "/api/v1/dashboards").cookie(&admin))
        .await
        .json();
    assert_eq!(theirs, json!([]), "even admins only see their own");
    for r in [
        app.send(req(Method::GET, &format!("/api/v1/dashboards/{id}")).cookie(&admin))
            .await,
        patch(&app, &admin, &id, json!({ "name": "Taken" })).await,
        app.send(req(Method::DELETE, &format!("/api/v1/dashboards/{id}")).cookie(&admin))
            .await,
    ] {
        assert_eq!(r.status, StatusCode::NOT_FOUND);
    }
    let still = app
        .send(req(Method::GET, &format!("/api/v1/dashboards/{id}")).cookie(&viewer))
        .await
        .json();
    assert_eq!(still["name"], "Mine");
}

#[tokio::test]
async fn invalid_layouts_are_rejected() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let a = camera(&app, &admin, 1).await;
    for body in [
        json!({ "name": "" }),
        json!({ "name": "Big", "rows": 9, "cols": 1 }),
        json!({ "name": "Out", "rows": 2, "cols": 2, "tiles": [{ "position": 4, "camera_id": a }] }),
        json!({ "name": "Twice", "tiles": [{ "position": 0, "camera_id": a }, { "position": 1, "camera_id": a }] }),
        json!({ "name": "Same", "tiles": [{ "position": 0, "camera_id": a }, { "position": 0, "camera_id": uuid::Uuid::now_v7() }] }),
        json!({ "name": "Ghost", "tiles": [{ "position": 0, "camera_id": uuid::Uuid::now_v7() }] }),
    ] {
        let r = create(&app, &admin, body.clone()).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}: {}", r.text());
    }
}

#[tokio::test]
async fn demo_mode_seeds_sample_dashboards() {
    let app = TestApp::new().await;
    crane::demo::seed(&app.state.db, &*app.state.credentials)
        .await
        .unwrap();
    let viewer = crane::users::find_credentials(&app.state.db, "viewer")
        .await
        .unwrap()
        .unwrap();
    let list = serde_json::to_value(
        crane::dashboards::list(&app.state.db, viewer.user.id)
            .await
            .unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Site A cranes", "Gates and docks"]);
    assert_eq!(list[0]["tiles"].as_array().unwrap().len(), 8);
    assert_eq!(list[0]["is_default"], true);
}
