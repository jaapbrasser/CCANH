//! Groups, tags, favorites and bulk operations (SPEC §9, Phase 6 acceptance:
//! parent-group filters include descendants, bulk tagging works, favorites
//! are per user).

mod common;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use serde_json::{json, Value};

async fn post(app: &TestApp, cookie: &str, path: &str, body: Value) -> common::Response {
    app.send(req(Method::POST, path).cookie(cookie).json(body))
        .await
}

async fn id_of(app: &TestApp, cookie: &str, path: &str, body: Value) -> String {
    let r = post(app, cookie, path, body).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["id"].as_str().unwrap().to_owned()
}

async fn camera(app: &TestApp, cookie: &str, n: u8, extra: Value) -> String {
    let mut body = json!({
        "name": format!("Camera {n}"), "host": format!("192.0.2.{n}"), "monitoring_enabled": false
    });
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    id_of(app, cookie, "/api/v1/cameras", body).await
}

async fn names(app: &TestApp, cookie: &str, query: &str) -> Vec<String> {
    let r = app
        .send(req(Method::GET, &format!("/api/v1/cameras?{query}")).cookie(cookie))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let mut v: Vec<String> = r.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn parent_group_filter_includes_descendants() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let site = id_of(&app, &admin, "/api/v1/groups", json!({ "name": "Site A" })).await;
    let building = id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Building 1", "parent_id": site }),
    )
    .await;
    let crane = id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Crane 1", "parent_id": building }),
    )
    .await;
    let other = id_of(&app, &admin, "/api/v1/groups", json!({ "name": "Site B" })).await;

    camera(&app, &admin, 1, json!({ "group_ids": [crane] })).await;
    camera(&app, &admin, 2, json!({ "group_ids": [building] })).await;
    camera(&app, &admin, 3, json!({ "group_ids": [other] })).await;
    camera(&app, &admin, 4, json!({})).await;

    assert_eq!(
        names(&app, &admin, &format!("group_id={site}")).await,
        ["Camera 1", "Camera 2"]
    );
    assert_eq!(
        names(&app, &admin, &format!("group_id={crane}")).await,
        ["Camera 1"]
    );
    let list = app
        .send(req(Method::GET, "/api/v1/cameras?q=Camera%201").cookie(&admin))
        .await
        .json();
    assert_eq!(list["items"][0]["groups"][0]["name"], "Crane 1");

    // Sibling names are unique ignoring case; other levels are fine.
    let dup = post(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "building 1", "parent_id": site }),
    )
    .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Building 1" }),
    )
    .await;

    // No cycles: a group can't move under its own descendant.
    let cycle = app
        .send(
            req(Method::PATCH, &format!("/api/v1/groups/{site}"))
                .cookie(&admin)
                .json(json!({ "parent_id": crane })),
        )
        .await;
    assert_eq!(cycle.status, StatusCode::BAD_REQUEST, "{}", cycle.text());
    let moved = app
        .send(
            req(Method::PATCH, &format!("/api/v1/groups/{crane}"))
                .cookie(&admin)
                .json(json!({ "parent_id": null })),
        )
        .await;
    assert_eq!(moved.json()["parent_id"], Value::Null);
    assert_eq!(
        names(&app, &admin, &format!("group_id={site}")).await,
        ["Camera 2"],
        "moved out of the tree"
    );
}

#[tokio::test]
async fn deleting_a_group_moves_up_or_detaches() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let site = id_of(&app, &admin, "/api/v1/groups", json!({ "name": "Site" })).await;
    let mid = id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Mid", "parent_id": site }),
    )
    .await;
    let leaf = id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Leaf", "parent_id": mid }),
    )
    .await;
    camera(&app, &admin, 1, json!({ "group_ids": [mid] })).await;

    let no_mode = app
        .send(req(Method::DELETE, &format!("/api/v1/groups/{mid}")).cookie(&admin))
        .await;
    assert_eq!(
        no_mode.status,
        StatusCode::BAD_REQUEST,
        "a mode is required"
    );

    let del = app
        .send(
            req(
                Method::DELETE,
                &format!("/api/v1/groups/{mid}?mode=move_up"),
            )
            .cookie(&admin),
        )
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text());
    let groups = app
        .send(req(Method::GET, "/api/v1/groups").cookie(&admin))
        .await
        .json();
    let leaf_row = groups
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == leaf.as_str())
        .unwrap();
    assert_eq!(leaf_row["parent_id"], site.as_str(), "subgroup moved up");
    assert_eq!(
        names(&app, &admin, &format!("group_id={site}")).await,
        ["Camera 1"],
        "camera moved up to the parent"
    );

    let del = app
        .send(
            req(
                Method::DELETE,
                &format!("/api/v1/groups/{site}?mode=detach"),
            )
            .cookie(&admin),
        )
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let groups = app
        .send(req(Method::GET, "/api/v1/groups").cookie(&admin))
        .await
        .json();
    assert_eq!(groups.as_array().unwrap().len(), 1);
    assert_eq!(groups[0]["parent_id"], Value::Null, "leaf is now top-level");
    assert_eq!(
        names(&app, &admin, "").await,
        ["Camera 1"],
        "cameras are kept"
    );
    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=group.delete").cookie(&admin))
        .await;
    assert_eq!(audit.json()["total"], 2);
}

#[tokio::test]
async fn bulk_tagging_and_tag_rules() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let outdoor = id_of(&app, &admin, "/api/v1/tags", json!({ "name": "Outdoor" })).await;
    assert_eq!(
        post(&app, &admin, "/api/v1/tags", json!({ "name": "OUTDOOR" }))
            .await
            .status,
        StatusCode::CONFLICT,
        "tag names ignore case"
    );
    let ids = [
        camera(&app, &admin, 1, json!({})).await,
        camera(&app, &admin, 2, json!({})).await,
        camera(&app, &admin, 3, json!({})).await,
    ];

    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": ids, "action": { "type": "add_tags", "tag_ids": [outdoor] } }),
    )
    .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.text());
    let task = app
        .wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    assert_eq!(task["status"], "succeeded", "{task}");
    assert_eq!(task["result"]["processed"], 3);
    assert_eq!(
        names(&app, &admin, &format!("tag_id={outdoor}")).await,
        ["Camera 1", "Camera 2", "Camera 3"]
    );
    let tags = app
        .send(req(Method::GET, "/api/v1/tags").cookie(&admin))
        .await
        .json();
    assert_eq!(tags[0]["camera_count"], 3);

    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": [ids[0]], "action": { "type": "remove_tags", "tag_ids": [outdoor] } }),
    )
    .await;
    app.wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    assert_eq!(
        names(&app, &admin, &format!("tag_id={outdoor}")).await,
        ["Camera 2", "Camera 3"]
    );

    // Unknown tags or cameras are rejected before any work starts.
    let bad = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": ids, "action": { "type": "add_tags", "tag_ids": [uuid::Uuid::now_v7()] } }),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.bulk_update").cookie(&admin))
        .await;
    assert_eq!(audit.json()["total"], 2);

    // Renaming keeps assignments; deleting removes the tag from cameras.
    let renamed = app
        .send(
            req(Method::PATCH, &format!("/api/v1/tags/{outdoor}"))
                .cookie(&admin)
                .json(json!({ "name": "Exterior" })),
        )
        .await;
    assert_eq!(renamed.json()["name"], "Exterior");
    let cam = app
        .send(req(Method::GET, &format!("/api/v1/cameras/{}", ids[1])).cookie(&admin))
        .await
        .json();
    assert_eq!(cam["tags"][0]["name"], "Exterior");
    app.send(req(Method::DELETE, &format!("/api/v1/tags/{outdoor}")).cookie(&admin))
        .await;
    let cam = app
        .send(req(Method::GET, &format!("/api/v1/cameras/{}", ids[1])).cookie(&admin))
        .await
        .json();
    assert_eq!(cam["tags"], json!([]));
}

#[tokio::test]
async fn bulk_monitoring_and_delete() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let ids = [
        camera(&app, &admin, 1, json!({})).await,
        camera(&app, &admin, 2, json!({})).await,
        camera(&app, &admin, 3, json!({})).await,
    ];
    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": [ids[0], ids[1]], "action": { "type": "set_monitoring", "enabled": true } }),
    )
    .await;
    app.wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    let list = app
        .send(req(Method::GET, "/api/v1/cameras").cookie(&admin))
        .await
        .json();
    let enabled: Vec<bool> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["monitoring_enabled"].as_bool().unwrap())
        .collect();
    assert_eq!(enabled, [true, true, false]);

    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": [ids[0], ids[2]], "action": { "type": "delete" } }),
    )
    .await;
    let task = app
        .wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    assert_eq!(task["status"], "succeeded", "{task}");
    assert_eq!(names(&app, &admin, "").await, ["Camera 2"]);
    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.delete").cookie(&admin))
        .await;
    assert_eq!(audit.json()["total"], 2, "each deleted camera is audited");

    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    let denied = post(
        &app,
        &viewer,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": [ids[1]], "action": { "type": "delete" } }),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let too_many = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({ "camera_ids": [], "action": { "type": "delete" } }),
    )
    .await;
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn favorites_are_per_user() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    let a = camera(&app, &admin, 1, json!({})).await;
    let b = camera(&app, &admin, 2, json!({})).await;

    for (cookie, cam) in [(&viewer, &a), (&admin, &b)] {
        let r = app
            .send(req(Method::PUT, &format!("/api/v1/favorites/{cam}")).cookie(cookie))
            .await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    }
    // Idempotent.
    app.send(req(Method::PUT, &format!("/api/v1/favorites/{a}")).cookie(&viewer))
        .await;

    let mine = app
        .send(req(Method::GET, "/api/v1/favorites").cookie(&viewer))
        .await
        .json();
    assert_eq!(mine, json!([a]));
    assert_eq!(names(&app, &viewer, "favorites=true").await, ["Camera 1"]);
    assert_eq!(names(&app, &admin, "favorites=true").await, ["Camera 2"]);

    app.send(req(Method::DELETE, &format!("/api/v1/favorites/{a}")).cookie(&viewer))
        .await;
    assert!(names(&app, &viewer, "favorites=true").await.is_empty());
    assert_eq!(names(&app, &admin, "favorites=true").await, ["Camera 2"]);

    let missing = app
        .send(
            req(
                Method::PUT,
                &format!("/api/v1/favorites/{}", uuid::Uuid::now_v7()),
            )
            .cookie(&viewer),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_settings_by_group_and_tag() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let site = id_of(&app, &admin, "/api/v1/groups", json!({ "name": "Site" })).await;
    let crane = id_of(
        &app,
        &admin,
        "/api/v1/groups",
        json!({ "name": "Crane", "parent_id": site }),
    )
    .await;
    let old = id_of(
        &app,
        &admin,
        "/api/v1/tags",
        json!({ "name": "Old firmware" }),
    )
    .await;
    let in_site = camera(&app, &admin, 1, json!({ "group_ids": [site] })).await;
    let in_crane = camera(&app, &admin, 2, json!({ "group_ids": [crane] })).await;
    let tagged = camera(
        &app,
        &admin,
        3,
        json!({ "tag_ids": [old], "scheme": "https", "tls_mode": "strict" }),
    )
    .await;
    let untouched = camera(&app, &admin, 4, json!({})).await;
    let set = id_of(
        &app,
        &admin,
        "/api/v1/credential-sets",
        json!({ "name": "Site viewer", "username": "viewer", "password": "s3cret-pw" }),
    )
    .await;

    // The group (with its subgroup) plus the tag: three cameras.
    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({
            "group_id": site,
            "tag_id": old,
            "action": { "type": "update_settings", "settings": {
                "credentials": { "type": "set", "id": set },
                "allow_basic_over_http": false,
                "tls_mode": "insecure_skip_verify",
            } },
        }),
    )
    .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.text());
    let task = app
        .wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    assert_eq!(task["status"], "succeeded", "{task}");
    assert_eq!(task["result"]["processed"], 3, "{task}");
    assert_eq!(task["result"]["skipped"], json!([]));

    let get = |id: &str| {
        let (app, admin) = (&app, &admin);
        let id = id.to_owned();
        async move {
            app.send(req(Method::GET, &format!("/api/v1/cameras/{id}")).cookie(admin))
                .await
                .json()
        }
    };
    for id in [&in_site, &in_crane, &tagged] {
        let c = get(id).await;
        assert_eq!(c["credential_set"]["id"], set.as_str(), "{c}");
        assert_eq!(c["allow_basic_over_http"], false);
    }
    // Only the HTTPS camera gets a certificate mode; HTTP ones still have none.
    assert_eq!(get(&tagged).await["tls_mode"], "insecure_skip_verify");
    assert_eq!(get(&in_site).await["tls_mode"], Value::Null);
    let u = get(&untouched).await;
    assert_eq!(u["has_credentials"], false);
    assert_eq!(u["allow_basic_over_http"], true);

    // Subgroups can be left out.
    let started = post(
        &app,
        &admin,
        "/api/v1/cameras/bulk",
        json!({
            "group_id": site, "include_subgroups": false,
            "action": { "type": "update_settings", "settings": { "port": 8080 } },
        }),
    )
    .await;
    let task = app
        .wait_task(&admin, started.json()["id"].as_str().unwrap())
        .await;
    assert_eq!(task["result"]["processed"], 1);
    assert_eq!(get(&in_site).await["port"], 8080);
    assert_eq!(get(&in_crane).await["port"], 80);

    // Audit: one entry per camera plus a summary; never the password.
    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.").cookie(&admin))
        .await;
    assert!(!audit.text().contains("s3cret-pw"));
    let summary = app
        .send(req(Method::GET, "/api/v1/audit?action=camera.bulk_update").cookie(&admin))
        .await
        .json();
    assert_eq!(summary["total"], 2);

    // Nothing to change, a pin, or an empty selection: rejected up front.
    for (body, why) in [
        (
            json!({ "camera_ids": [untouched], "action": { "type": "update_settings", "settings": {} } }),
            "no settings",
        ),
        (
            json!({ "camera_ids": [untouched], "action": { "type": "update_settings", "settings": { "tls_mode": "pinned" } } }),
            "pins are per camera",
        ),
        (
            json!({ "action": { "type": "set_monitoring", "enabled": true } }),
            "empty selection",
        ),
    ] {
        let r = post(&app, &admin, "/api/v1/cameras/bulk", body).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{why}: {}", r.text());
    }
}
