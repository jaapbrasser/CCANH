//! Proves passwords never reach log output (SPEC §4.6, §11.1, §15.3).
//!
//! Its own test binary, because it installs a global tracing subscriber at TRACE
//! level that captures everything, including sqlx and tower-http logs.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::http::Method;
use common::{req, TestApp};
use crane::auth::Role;
use serde_json::json;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn passwords_never_appear_in_logs() {
    let capture = Capture::default();
    let writer = capture.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .init();

    let secrets = [
        "first distinctive secret",
        "second distinctive secret",
        "third distinctive secret",
        "fourth distinctive secret",
        "fifth-camera-secret",
        "sixth-camera-secret",
    ];

    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;

    // Failed login with a secret-looking password.
    app.send(req(Method::POST, "/api/v1/auth/login").json(json!({
        "username": "admin", "password": secrets[0]
    })))
    .await;
    // Create a user, reset their password, and have them change it.
    let created = app
        .send(
            req(Method::POST, "/api/v1/users")
                .cookie(&admin)
                .json(json!({
                    "username": "bob", "password": secrets[1], "role": "viewer"
                })),
        )
        .await;
    let id = created.json()["id"].as_str().unwrap().to_owned();
    app.send(
        req(Method::POST, &format!("/api/v1/users/{id}/password"))
            .cookie(&admin)
            .json(json!({ "new_password": secrets[2] })),
    )
    .await;
    let bob = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "bob", "password": secrets[2]
        })))
        .await
        .session_cookie()
        .unwrap();
    app.send(
        req(Method::POST, "/api/v1/auth/password")
            .cookie(&bob)
            .json(json!({
                "current_password": secrets[2], "new_password": secrets[3]
            })),
    )
    .await;
    // A setup attempt with a secret token.
    app.send(req(Method::POST, "/api/v1/setup").json(json!({
        "token": "secret-setup-token-value", "username": "x", "password": secrets[0]
    })))
    .await;

    // Camera credentials: a shared set, inline credentials, and a Digest-
    // authenticated request to a mock AXIS camera via test-connection.
    let camera = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path("/axis-cgi/basicdeviceinfo.cgi"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_raw(
            r#"{"apiVersion":"1.0","data":{"propertyList":{"Brand":"AXIS","ProdNbr":"M1"}}}"#,
            "application/json",
        ))
        .mount(&camera)
        .await;
    wiremock::Mock::given(wiremock::matchers::path("/axis-cgi/streamprofile.cgi"))
        .respond_with(wiremock::ResponseTemplate::new(401).insert_header(
            "WWW-Authenticate",
            r#"Digest realm="AXIS_T", nonce="n0nce", algorithm=MD5, qop="auth""#,
        ))
        .mount(&camera)
        .await;
    app.send(
        req(Method::POST, "/api/v1/credential-sets")
            .cookie(&admin)
            .json(json!({
                "name": "Shared", "username": "svc", "password": secrets[4]
            })),
    )
    .await;
    let addr = camera.address();
    app.send(
        req(Method::POST, "/api/v1/cameras/test-connection")
            .cookie(&admin)
            .json(json!({
                "host": addr.ip().to_string(), "port": addr.port(), "scheme": "http",
                "credentials": { "type": "inline", "username": "root", "password": secrets[5] }
            })),
    )
    .await;
    app.send(
        req(Method::POST, "/api/v1/cameras")
            .cookie(&admin)
            .json(json!({
                "name": "Cam", "host": "192.0.2.99",
                "credentials": { "type": "inline", "username": "root", "password": secrets[5] }
            })),
    )
    .await;

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        !logs.is_empty(),
        "nothing was captured; the test is not effective"
    );
    for secret in secrets.iter().chain(["secret-setup-token-value"].iter()) {
        assert!(!logs.contains(secret), "secret '{secret}' found in logs");
    }
    assert!(!logs.contains("$argon2"), "password hash found in logs");
    assert!(
        !logs.contains("crane_session="),
        "session cookie found in logs"
    );
}
