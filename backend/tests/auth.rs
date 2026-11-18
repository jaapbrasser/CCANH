//! Authentication, authorization, CSRF, rate limiting, setup, user admin and
//! audit tests, all through the HTTP API.

mod common;

use axum::http::{header, Method, StatusCode};
use common::{req, TestApp, PASSWORD};
use crane::auth::Role;
use serde_json::json;

// ---------------------------------------------------------------------------
// Role enforcement, generated from the OpenAPI route table (SPEC §15.3)
// ---------------------------------------------------------------------------

fn required_role(op: &serde_json::Value) -> String {
    let text = format!(
        "{} {}",
        op["summary"].as_str().unwrap_or_default(),
        op["description"].as_str().unwrap_or_default()
    );
    let after = text
        .split("Required role:")
        .nth(1)
        .unwrap_or_else(|| panic!("operation without required role: {text}"));
    after
        .trim_start()
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

#[tokio::test]
async fn every_protected_route_enforces_its_role() {
    let app = TestApp::new().await;
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    let doc = serde_json::to_value(crane::api::openapi()).unwrap();
    let dummy_id = uuid::Uuid::now_v7().to_string();

    let mut checked_admin = 0;
    let mut checked_viewer = 0;
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let role = required_role(op);
            let method: Method = method.to_uppercase().parse().unwrap();
            let concrete = path.replace("{id}", &dummy_id);
            let request = || {
                let r = req(method.clone(), &concrete);
                if matches!(method, Method::POST | Method::PUT | Method::PATCH) {
                    r.json(json!({}))
                } else {
                    r
                }
            };

            match role.as_str() {
                "none" => continue,
                "viewer" | "admin" => {
                    let res = app.send(request()).await;
                    assert_eq!(
                        res.status,
                        StatusCode::UNAUTHORIZED,
                        "{method} {path} must reject anonymous requests: {}",
                        res.text()
                    );
                }
                other => panic!("{method} {path}: unknown role '{other}'"),
            }

            if role == "admin" {
                let res = app.send(request().cookie(&viewer)).await;
                assert_eq!(
                    res.status,
                    StatusCode::FORBIDDEN,
                    "{method} {path} must reject viewers: {}",
                    res.text()
                );
                checked_admin += 1;
            } else {
                checked_viewer += 1;
            }
        }
    }
    assert!(
        checked_admin >= 6,
        "only {checked_admin} admin routes checked"
    );
    assert!(
        checked_viewer >= 3,
        "only {checked_viewer} viewer routes checked"
    );
}

// ---------------------------------------------------------------------------
// Login, logout, sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_sets_hardened_cookie_and_me_works() {
    let app = TestApp::new().await;
    app.user("alice", Role::Viewer).await;
    let res = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "ALICE", "password": PASSWORD
        })))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json()["username"], "alice");
    assert_eq!(res.json()["role"], "viewer");

    let set_cookie = res.set_cookie_header();
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(set_cookie.contains("SameSite=Strict"), "{set_cookie}");
    assert!(set_cookie.contains("Path=/"), "{set_cookie}");
    assert!(
        !set_cookie.contains("Secure"),
        "no Secure without TLS: {set_cookie}"
    );

    let cookie = res.session_cookie().unwrap();
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&cookie))
        .await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json()["username"], "alice");
}

#[tokio::test]
async fn cookie_is_secure_when_tls_is_enabled() {
    let mut config = crane::config::Config::default();
    config.server.tls.mode = crane::config::TlsMode::SelfSigned;
    let app = TestApp::with_config(config).await;
    app.user("alice", Role::Viewer).await;
    let res = app
        .send(
            req(Method::POST, "/api/v1/auth/login")
                .origin(Some("https://localhost"))
                .json(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text());
    assert!(res.set_cookie_header().contains("Secure"));
}

#[tokio::test]
async fn wrong_password_and_unknown_user_look_identical() {
    let app = TestApp::new().await;
    app.user("alice", Role::Viewer).await;
    let wrong = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "alice", "password": "not the right password"
        })))
        .await;
    let unknown = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "nobody", "password": "not the right password"
        })))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong.body, unknown.body);
    assert!(wrong.session_cookie().is_none());
}

#[tokio::test]
async fn disabled_user_cannot_log_in() {
    let app = TestApp::new().await;
    let user = app.user("alice", Role::Viewer).await;
    crane::users::update(&app.state.db, user.id, None, Some(true))
        .await
        .unwrap();
    let res = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "alice", "password": PASSWORD
        })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_ends_the_session() {
    let app = TestApp::new().await;
    let (_, cookie) = app.signed_in("alice", Role::Viewer).await;
    let res = app
        .send(req(Method::POST, "/api/v1/auth/logout").cookie(&cookie))
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(res.set_cookie_header().contains("Max-Age=0"));
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&cookie))
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn idle_session_expires() {
    let app = TestApp::new().await;
    let (_, cookie) = app.signed_in("alice", Role::Viewer).await;
    // Pretend the last request was 9 hours ago (idle timeout is 8 h).
    let nine_hours_ago = time::OffsetDateTime::now_utc().unix_timestamp() - 9 * 3600;
    sqlx::query("UPDATE sessions SET last_seen_at = ?")
        .bind(nine_hours_ago)
        .execute(&app.state.db)
        .await
        .unwrap();
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&cookie))
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn absolute_session_lifetime_is_enforced() {
    let app = TestApp::new().await;
    let (_, cookie) = app.signed_in("alice", Role::Viewer).await;
    sqlx::query("UPDATE sessions SET expires_at = ?")
        .bind(time::OffsetDateTime::now_utc().unix_timestamp() - 1)
        .execute(&app.state.db)
        .await
        .unwrap();
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&cookie))
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn forged_cookie_is_rejected() {
    let app = TestApp::new().await;
    app.user("alice", Role::Viewer).await;
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie("crane_session=forged"))
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repeated_failures_are_rate_limited_even_with_correct_password() {
    let app = TestApp::new().await;
    app.user("alice", Role::Viewer).await;
    let attempt = |password: &'static str| {
        req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "alice", "password": password
        }))
    };
    for _ in 0..6 {
        let res = app.send(attempt("not the right password")).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }
    let res = app.send(attempt(PASSWORD)).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.json()["code"], "rate_limited");
    let retry: u64 = res.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry >= 1);

    // Other users are unaffected (no IP in in-process tests).
    app.user("bob", Role::Viewer).await;
    app.login("bob").await;
}

// ---------------------------------------------------------------------------
// CSRF
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_changing_requests_need_a_matching_origin() {
    let app = TestApp::new().await;
    let (_, cookie) = app.signed_in("admin", Role::Admin).await;
    let create = || {
        req(Method::POST, "/api/v1/users")
            .cookie(&cookie)
            .json(json!({
                "username": "eve", "password": "long enough password", "role": "viewer"
            }))
    };

    let missing = app.send(create().origin(None)).await;
    assert_eq!(missing.status, StatusCode::FORBIDDEN);
    assert_eq!(missing.json()["code"], "csrf_rejected");

    let foreign = app.send(create().origin(Some("http://evil.example"))).await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);

    // Safe methods don't need Origin.
    let list = app
        .send(
            req(Method::GET, "/api/v1/users")
                .cookie(&cookie)
                .origin(None),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK);

    let ok = app.send(create()).await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text());
}

#[tokio::test]
async fn login_is_also_csrf_protected() {
    let app = TestApp::new().await;
    app.user("alice", Role::Viewer).await;
    let res = app
        .send(
            req(Method::POST, "/api/v1/auth/login")
                .origin(Some("http://evil.example"))
                .json(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// First-run setup
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setup_flow_creates_first_admin_once() {
    let app = TestApp::new().await;
    let status = app.send(req(Method::GET, "/api/v1/setup")).await;
    assert_eq!(status.json()["setup_required"], true);

    let token = app
        .state
        .issue_setup_token_if_needed()
        .await
        .unwrap()
        .unwrap();
    let body = |token: &str| json!({ "token": token, "username": "root", "password": "a strong first password" });

    let wrong = app
        .send(req(Method::POST, "/api/v1/setup").json(body("wrong-token")))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let res = app
        .send(req(Method::POST, "/api/v1/setup").json(body(&token)))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.text());
    assert_eq!(res.json()["role"], "admin");
    let cookie = res.session_cookie().unwrap();
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&cookie))
        .await;
    assert_eq!(me.json()["username"], "root");

    let status = app.send(req(Method::GET, "/api/v1/setup")).await;
    assert_eq!(status.json()["setup_required"], false);

    let again = app
        .send(req(Method::POST, "/api/v1/setup").json(body(&token)))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn no_setup_token_when_users_exist() {
    let app = TestApp::new().await;
    app.user("admin", Role::Admin).await;
    assert!(app
        .state
        .issue_setup_token_if_needed()
        .await
        .unwrap()
        .is_none());
}

// ---------------------------------------------------------------------------
// User administration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_manages_users() {
    let app = TestApp::new().await;
    let (admin, cookie) = app.signed_in("admin", Role::Admin).await;

    let created = app
        .send(
            req(Method::POST, "/api/v1/users")
                .cookie(&cookie)
                .json(json!({
                    "username": "bob", "password": "bob's long password", "role": "viewer"
                })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let bob_id = created.json()["id"].as_str().unwrap().to_owned();

    let dup = app
        .send(
            req(Method::POST, "/api/v1/users")
                .cookie(&cookie)
                .json(json!({
                    "username": "BOB", "password": "bob's long password", "role": "viewer"
                })),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);

    let weak = app
        .send(
            req(Method::POST, "/api/v1/users")
                .cookie(&cookie)
                .json(json!({
                    "username": "carol", "password": "short", "role": "viewer"
                })),
        )
        .await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);

    let list = app
        .send(
            req(
                Method::GET,
                "/api/v1/users?page=1&page_size=1&sort=username&order=desc",
            )
            .cookie(&cookie),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    assert_eq!(list.json()["total"], 2);
    assert_eq!(list.json()["items"][0]["username"], "bob");

    let promoted = app
        .send(
            req(Method::PATCH, &format!("/api/v1/users/{bob_id}"))
                .cookie(&cookie)
                .json(json!({ "role": "admin" })),
        )
        .await;
    assert_eq!(promoted.json()["role"], "admin");

    let self_delete = app
        .send(req(Method::DELETE, &format!("/api/v1/users/{}", admin.id)).cookie(&cookie))
        .await;
    assert_eq!(self_delete.status, StatusCode::CONFLICT);

    let deleted = app
        .send(req(Method::DELETE, &format!("/api/v1/users/{bob_id}")).cookie(&cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = app
        .send(req(Method::GET, &format!("/api/v1/users/{bob_id}")).cookie(&cookie))
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn last_admin_cannot_be_demoted_via_api() {
    let app = TestApp::new().await;
    let (admin, cookie) = app.signed_in("admin", Role::Admin).await;
    let res = app
        .send(
            req(Method::PATCH, &format!("/api/v1/users/{}", admin.id))
                .cookie(&cookie)
                .json(json!({ "role": "viewer" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert!(res.json()["detail"]
        .as_str()
        .unwrap()
        .contains("no enabled admin"));
}

#[tokio::test]
async fn disabling_a_user_ends_their_sessions() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (bob, bob_cookie) = app.signed_in("bob", Role::Viewer).await;
    let res = app
        .send(
            req(Method::PATCH, &format!("/api/v1/users/{}", bob.id))
                .cookie(&admin)
                .json(json!({ "disabled": true })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let me = app
        .send(req(Method::GET, "/api/v1/auth/me").cookie(&bob_cookie))
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn role_change_takes_effect_on_existing_session() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (bob, bob_cookie) = app.signed_in("bob", Role::Admin).await;
    assert_eq!(
        app.send(req(Method::GET, "/api/v1/users").cookie(&bob_cookie))
            .await
            .status,
        StatusCode::OK
    );
    app.send(
        req(Method::PATCH, &format!("/api/v1/users/{}", bob.id))
            .cookie(&admin)
            .json(json!({ "role": "viewer" })),
    )
    .await;
    assert_eq!(
        app.send(req(Method::GET, "/api/v1/users").cookie(&bob_cookie))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_password_reset_ends_sessions_and_new_password_works() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let (bob, bob_cookie) = app.signed_in("bob", Role::Viewer).await;
    let res = app
        .send(
            req(Method::POST, &format!("/api/v1/users/{}/password", bob.id))
                .cookie(&admin)
                .json(json!({ "new_password": "bob's brand new password" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.send(req(Method::GET, "/api/v1/auth/me").cookie(&bob_cookie))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let login = app
        .send(req(Method::POST, "/api/v1/auth/login").json(json!({
            "username": "bob", "password": "bob's brand new password"
        })))
        .await;
    assert_eq!(login.status, StatusCode::OK);
}

#[tokio::test]
async fn change_own_password() {
    let app = TestApp::new().await;
    let (_, current) = app.signed_in("alice", Role::Viewer).await;
    let other = app.login("alice").await;

    let wrong = app
        .send(
            req(Method::POST, "/api/v1/auth/password")
                .cookie(&current)
                .json(json!({
                    "current_password": "not my password", "new_password": "a new long password"
                })),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let weak = app
        .send(
            req(Method::POST, "/api/v1/auth/password")
                .cookie(&current)
                .json(json!({
                    "current_password": PASSWORD, "new_password": "short"
                })),
        )
        .await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);

    let ok = app
        .send(
            req(Method::POST, "/api/v1/auth/password")
                .cookie(&current)
                .json(json!({
                    "current_password": PASSWORD, "new_password": "a new long password"
                })),
        )
        .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);

    // Current session survives, the other one is ended.
    assert_eq!(
        app.send(req(Method::GET, "/api/v1/auth/me").cookie(&current))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(req(Method::GET, "/api/v1/auth/me").cookie(&other))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

#[tokio::test]
async fn security_events_are_audited() {
    let app = TestApp::new().await;
    app.user("admin", Role::Admin).await;
    // A failed and a successful login.
    app.send(req(Method::POST, "/api/v1/auth/login").json(json!({
        "username": "admin", "password": "not the right password"
    })))
    .await;
    let cookie = app.login("admin").await;
    app.send(
        req(Method::POST, "/api/v1/users")
            .cookie(&cookie)
            .json(json!({
                "username": "bob", "password": "bob's long password", "role": "viewer"
            })),
    )
    .await;

    let res = app
        .send(req(Method::GET, "/api/v1/audit?action=auth.login").cookie(&cookie))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text());
    let items = res.json()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["outcome"], "success");
    assert_eq!(items[1]["outcome"], "failure");
    assert_eq!(items[1]["details"]["reason"], "bad_password");

    let res = app
        .send(req(Method::GET, "/api/v1/audit?action=user.").cookie(&cookie))
        .await;
    let items = res.json()["items"].as_array().unwrap().clone();
    assert_eq!(items[0]["action"], "user.create");
    assert_eq!(items[0]["details"]["username"], "bob");
}

// ---------------------------------------------------------------------------
// Secrets never leave the server
// ---------------------------------------------------------------------------

#[tokio::test]
async fn responses_never_contain_passwords_or_hashes() {
    let app = TestApp::new().await;
    let (_, cookie) = app.signed_in("admin", Role::Admin).await;
    let secret = "a very distinctive password";
    let mut bodies = Vec::new();
    let created = app
        .send(
            req(Method::POST, "/api/v1/users")
                .cookie(&cookie)
                .json(json!({
                    "username": "bob", "password": secret, "role": "viewer"
                })),
        )
        .await;
    let id = created.json()["id"].as_str().unwrap().to_owned();
    bodies.push(created.text());
    bodies.push(
        app.send(req(Method::GET, "/api/v1/users").cookie(&cookie))
            .await
            .text(),
    );
    bodies.push(
        app.send(req(Method::GET, &format!("/api/v1/users/{id}")).cookie(&cookie))
            .await
            .text(),
    );
    bodies.push(
        app.send(req(Method::GET, "/api/v1/audit").cookie(&cookie))
            .await
            .text(),
    );
    for body in bodies {
        assert!(!body.contains(secret), "password leaked: {body}");
        assert!(!body.contains("argon2"), "hash leaked: {body}");
        assert!(!body.contains("password_hash"), "hash field leaked: {body}");
    }
}
