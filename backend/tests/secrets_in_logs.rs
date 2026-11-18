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
