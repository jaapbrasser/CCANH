//! Credential sets, camera credentials and test-connection through the API.

mod common;

use axum::http::{Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::config::{Config, Mode};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SECRET: &str = "very-secret-camera-pw";

fn assert_no_secret(body: &str) {
    assert!(!body.contains(SECRET), "password leaked: {body}");
    assert!(!body.contains("aesgcm1"), "ciphertext leaked: {body}");
}

#[tokio::test]
async fn credential_sets_are_write_only_and_audited() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;

    let created = app
        .send(
            req(Method::POST, "/api/v1/credential-sets")
                .cookie(&admin)
                .json(json!({
                    "name": "Site A cameras", "username": "svc-crane", "password": SECRET
                })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    assert_no_secret(&created.text());
    assert!(
        !created.text().contains("svc-crane"),
        "usernames are write-only too"
    );
    let id = created.json()["id"].as_str().unwrap().to_owned();

    let dup = app
        .send(
            req(Method::POST, "/api/v1/credential-sets")
                .cookie(&admin)
                .json(json!({
                    "name": "site a CAMERAS", "username": "x", "password": "y"
                })),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);

    let half = app
        .send(
            req(Method::PATCH, &format!("/api/v1/credential-sets/{id}"))
                .cookie(&admin)
                .json(json!({ "password": "only-password" })),
        )
        .await;
    assert_eq!(half.status, StatusCode::BAD_REQUEST);

    // Use the set on a camera, then try to delete it.
    let cam = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Gate", "host": "192.0.2.70", "credentials": { "type": "set", "id": id }
                })),
        )
        .await;
    assert_eq!(cam.status, StatusCode::CREATED, "{}", cam.text());
    assert_eq!(cam.json()["has_credentials"], true);
    assert_eq!(cam.json()["credential_set"]["name"], "Site A cameras");

    let listed = app
        .send(req(Method::GET, "/api/v1/credential-sets").cookie(&admin))
        .await;
    assert_eq!(listed.json()[0]["camera_count"], 1);
    assert_no_secret(&listed.text());

    let in_use = app
        .send(req(Method::DELETE, &format!("/api/v1/credential-sets/{id}")).cookie(&admin))
        .await;
    assert_eq!(in_use.status, StatusCode::CONFLICT);

    let audit = app
        .send(req(Method::GET, "/api/v1/audit?action=credential_set.").cookie(&admin))
        .await;
    assert_eq!(audit.json()["items"][0]["action"], "credential_set.create");
    assert_no_secret(&audit.text());
}

#[tokio::test]
async fn inline_camera_credentials_are_private_and_never_returned() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let cam = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "Dock", "host": "192.0.2.71",
                    "credentials": { "type": "inline", "username": "root", "password": SECRET }
                })),
        )
        .await;
    assert_eq!(cam.status, StatusCode::CREATED, "{}", cam.text());
    assert_eq!(cam.json()["has_credentials"], true);
    assert!(cam.json()["credential_set"].is_null());
    assert_no_secret(&cam.text());
    let id = cam.json()["id"].as_str().unwrap().to_owned();

    for path in [
        format!("/api/v1/cameras/{id}"),
        "/api/v1/cameras".to_owned(),
        "/api/v1/credential-sets".to_owned(),
        "/api/v1/audit".to_owned(),
    ] {
        let res = app.send(req(Method::GET, &path).cookie(&admin)).await;
        assert_no_secret(&res.text());
    }
    // Private sets don't show up as shared sets.
    let sets = app
        .send(req(Method::GET, "/api/v1/credential-sets").cookie(&admin))
        .await;
    assert_eq!(sets.json().as_array().unwrap().len(), 0);

    // Stored encrypted: the raw database never contains the password.
    let raw: Vec<Vec<u8>> = sqlx::query_scalar("SELECT secret FROM credential_sets")
        .fetch_all(&app.state.db)
        .await
        .unwrap();
    assert_eq!(raw.len(), 1);
    assert!(!raw[0].windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));

    let bad_set = app
        .send(
            req(Method::POST, "/api/v1/cameras")
                .cookie(&admin)
                .json(json!({
                    "name": "X", "host": "192.0.2.72",
                    "credentials": { "type": "set", "id": uuid::Uuid::now_v7() }
                })),
        )
        .await;
    assert_eq!(bad_set.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_connection_in_demo_mode() {
    let app = TestApp::with_config(Config {
        mode: Mode::Demo,
        ..Config::default()
    })
    .await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let test = |host: &'static str| {
        req(Method::POST, "/api/v1/cameras/test-connection")
            .cookie(&admin)
            .json(json!({ "host": host }))
    };

    let ok = app.send(test("192.0.2.11")).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.json()["ok"], true);
    assert_eq!(ok.json()["device"]["model"], "AXIS Q6135-LE");
    assert!(!ok.json()["capabilities"].as_array().unwrap().is_empty());

    let auth = app.send(test("198.51.100.24")).await;
    assert_eq!(auth.json()["ok"], false);
    let steps = auth.json()["steps"].as_array().unwrap().clone();
    assert_eq!(steps[0]["name"], "reachability");
    assert_eq!(steps[0]["ok"], true);
    assert_eq!(steps[1]["name"], "credentials");
    assert_eq!(steps[1]["error_category"], "auth_failed");

    let offline = app.send(test("192.0.2.34")).await;
    let steps = offline.json()["steps"].as_array().unwrap().clone();
    assert_eq!(steps[0]["error_category"], "unreachable");
    assert_eq!(steps[1]["skipped"], true);

    let invalid = app
        .send(
            req(Method::POST, "/api/v1/cameras/test-connection")
                .cookie(&admin)
                .json(json!({ "host": "http://admin:pw@cam/" })),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
}

/// A minimal AXIS device: anonymous identity, Digest-protected stream profiles.
struct Gate;
impl Respond for Gate {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        if req.headers.contains_key("authorization") {
            // The provider's Digest maths is unit-tested; here we only check that
            // credentials were sent after the challenge.
            ResponseTemplate::new(200).set_body_raw(
                r#"{"apiVersion":"1.0","method":"list","data":{"streamProfile":[{"name":"main","description":"","parameters":"resolution=1280x720"}]}}"#,
                "application/json",
            )
        } else {
            ResponseTemplate::new(401).insert_header(
                "WWW-Authenticate",
                r#"Digest realm="AXIS_TEST", nonce="abc123", algorithm=MD5, qop="auth""#,
            )
        }
    }
}

#[tokio::test]
async fn test_connection_against_axis_device_and_reuses_stored_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/axis-cgi/basicdeviceinfo.cgi"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"apiVersion":"1.0","data":{"propertyList":{"Brand":"AXIS","ProdNbr":"M3086-V","Version":"11.11.73","SerialNumber":"ACCC8E000001"}}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/axis-cgi/streamprofile.cgi"))
        .respond_with(Gate)
        .mount(&server)
        .await;

    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let addr = server.address();
    let body = |creds: serde_json::Value| {
        json!({
            "host": addr.ip().to_string(), "port": addr.port(), "scheme": "http",
            "credentials": creds,
        })
    };

    let res = app
        .send(
            req(Method::POST, "/api/v1/cameras/test-connection")
                .cookie(&admin)
                .json(body(
                    json!({ "type": "inline", "username": "viewer", "password": SECRET }),
                )),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let j = res.json();
    assert_eq!(j["device"]["model"], "AXIS M3086-V");
    assert_eq!(j["steps"][1]["name"], "credentials");
    assert_eq!(j["steps"][1]["ok"], true, "{j}");
    assert_eq!(j["stream_profiles"][0]["width"], 1280);
    assert_no_secret(&res.text());

    // Without credentials the camera's challenge is reported clearly.
    let res = app
        .send(
            req(Method::POST, "/api/v1/cameras/test-connection")
                .cookie(&admin)
                .json(body(json!({ "type": "none" }))),
        )
        .await;
    assert_eq!(res.json()["steps"][1]["error_category"], "auth_failed");

    // Saved camera: test again without re-entering the password.
    let cam = app
        .send(req(Method::POST, "/api/v1/cameras").cookie(&admin).json(json!({
            "name": "Mock", "host": addr.ip().to_string(), "port": addr.port(), "scheme": "http",
            "credentials": { "type": "inline", "username": "viewer", "password": SECRET }
        })))
        .await;
    let id = cam.json()["id"].as_str().unwrap().to_owned();
    let res = app
        .send(
            req(Method::POST, "/api/v1/cameras/test-connection")
                .cookie(&admin)
                .json(json!({
                    "host": addr.ip().to_string(), "port": addr.port(), "scheme": "http",
                    "camera_id": id,
                })),
        )
        .await;
    assert_eq!(res.json()["steps"][1]["ok"], true, "{}", res.text());
}
