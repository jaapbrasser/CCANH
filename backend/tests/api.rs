//! In-process HTTP tests for the platform basics: health, errors, headers,
//! static files, OpenAPI and serving.

mod common;

use std::time::Duration;

use axum::http::{header, Method, StatusCode};
use common::{req, TestApp};
use crane::auth::Role;
use crane::config::Config;

#[tokio::test]
async fn health_reports_ok() {
    let app = TestApp::new().await;
    let res = app.send(req(Method::GET, "/api/v1/health")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.headers[header::CONTENT_TYPE], "application/json");
    let json = res.json();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["mode"], "production");
    assert_eq!(json["checks"]["database"]["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn health_degrades_when_database_is_closed() {
    let app = TestApp::new().await;
    app.state.db.close().await;
    let res = app.send(req(Method::GET, "/api/v1/health")).await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(res.json()["status"], "degraded");
}

#[tokio::test]
async fn unknown_api_path_is_problem_json_404() {
    let app = TestApp::new().await;
    let res = app.send(req(Method::GET, "/api/v1/does-not-exist")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(
        res.headers[header::CONTENT_TYPE],
        "application/problem+json"
    );
    assert_eq!(res.json()["type"], "urn:crane:problem:not_found");
}

#[tokio::test]
async fn malformed_json_is_problem_json_400() {
    let app = TestApp::new().await;
    let res = app
        .send(req(Method::POST, "/api/v1/auth/login").json(serde_json::json!({ "username": 5 })))
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        res.headers[header::CONTENT_TYPE],
        "application/problem+json"
    );
}

#[tokio::test]
async fn security_headers_are_set() {
    let app = TestApp::new().await;
    for path in ["/api/v1/health", "/", "/api/nope"] {
        let res = app.send(req(Method::GET, path)).await;
        let csp = res.headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'"), "{path}");
        assert!(csp.contains("script-src 'self'"), "{path}");
        assert!(!csp.contains("unsafe-inline"), "{path}");
        assert_eq!(
            res.headers[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff",
            "{path}"
        );
        assert_eq!(
            res.headers[header::REFERRER_POLICY],
            "no-referrer",
            "{path}"
        );
    }
}

#[tokio::test]
async fn spa_routes_serve_html() {
    let app = TestApp::new().await;
    for path in ["/", "/users", "/login"] {
        let res = app.send(req(Method::GET, path)).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert!(res.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert!(res.text().contains("<html"), "{path}");
    }
}

#[tokio::test]
async fn missing_static_file_is_404() {
    let app = TestApp::new().await;
    let res = app.send(req(Method::GET, "/assets/nope-123.js")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn openapi_is_admin_only_and_matches_cli_output() {
    let app = TestApp::new().await;
    assert_eq!(
        app.send(req(Method::GET, "/api/openapi.json")).await.status,
        StatusCode::UNAUTHORIZED
    );
    let (_, viewer) = app.signed_in("viewer", Role::Viewer).await;
    assert_eq!(
        app.send(req(Method::GET, "/api/openapi.json").cookie(&viewer))
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let res = app
        .send(req(Method::GET, "/api/openapi.json").cookie(&admin))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let offline = serde_json::to_value(crane::api::openapi()).unwrap();
    assert_eq!(
        res.json(),
        offline,
        "served and --print-openapi documents differ"
    );
}

#[tokio::test]
async fn swagger_redirects_signed_out_browsers_to_login() {
    let app = TestApp::new().await;
    let res = app.send(req(Method::GET, "/api/docs/")).await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert_eq!(res.headers[header::LOCATION], "/login?next=/api/docs/");
}

#[tokio::test]
async fn swagger_ui_is_served_to_admins_with_its_own_csp() {
    let app = TestApp::new().await;
    let (_, admin) = app.signed_in("admin", Role::Admin).await;
    let res = app
        .send(req(Method::GET, "/api/docs/").cookie(&admin))
        .await;
    assert_eq!(res.status, StatusCode::OK);
    let csp = res.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
}

#[tokio::test]
async fn public_api_docs_setting_opens_docs() {
    let mut config = Config::default();
    config.server.public_api_docs = true;
    let app = TestApp::with_config(config).await;
    assert_eq!(
        app.send(req(Method::GET, "/api/openapi.json")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn every_operation_declares_required_role() {
    let doc = serde_json::to_value(crane::api::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    assert!(!paths.is_empty());
    for (path, item) in paths {
        for (method, op) in item.as_object().unwrap() {
            let text = format!(
                "{} {}",
                op["summary"].as_str().unwrap_or_default(),
                op["description"].as_str().unwrap_or_default()
            );
            assert!(
                text.contains("Required role:"),
                "{method} {path} does not declare its required role"
            );
        }
    }
}

#[test]
fn operation_ids_are_unique_and_explicit() {
    let doc = serde_json::to_value(crane::api::openapi()).unwrap();
    let mut seen = std::collections::HashSet::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let id = op["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
            assert!(seen.insert(id.to_owned()), "duplicate operationId '{id}'");
        }
    }
}

#[test]
fn openapi_has_no_dangling_refs() {
    fn walk(value: &serde_json::Value, refs: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(r)) = map.get("$ref") {
                    refs.push(r.clone());
                }
                map.values().for_each(|v| walk(v, refs));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, refs)),
            _ => {}
        }
    }
    let doc = serde_json::to_value(crane::api::openapi()).unwrap();
    let mut refs = Vec::new();
    walk(&doc, &mut refs);
    assert!(!refs.is_empty());
    for r in refs {
        let name = r
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("unexpected $ref form: {r}"));
        assert!(
            doc["components"]["schemas"][name].is_object(),
            "dangling $ref: {r}"
        );
    }
}

#[tokio::test]
async fn serves_over_tcp_and_shuts_down_gracefully() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let app = TestApp::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crane::serve(listener, app.state, None, shutdown.clone()));

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server did not shut down")
        .unwrap()
        .unwrap();
}
