//! In-process HTTP tests against the full router.

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crane::config::Config;
use crane::{storage, AppState};

async fn state() -> AppState {
    let db = storage::connect_in_memory().await.unwrap();
    AppState::new(Config::default(), db)
}

async fn get(path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let app = crane::app(state().await);
    let response = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, body)
}

#[tokio::test]
async fn health_reports_ok() {
    let (status, headers, body) = get("/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["mode"], "production");
    assert_eq!(json["checks"]["database"]["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn health_degrades_when_database_is_closed() {
    let state = state().await;
    state.db.close().await;
    let response = crane::app(state)
        .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "degraded");
}

#[tokio::test]
async fn unknown_api_path_is_problem_json_404() {
    let (status, headers, body) = get("/api/v1/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers[header::CONTENT_TYPE], "application/problem+json");
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["type"], "urn:crane:problem:not_found");
    assert_eq!(json["status"], 404);
}

#[tokio::test]
async fn security_headers_are_set() {
    for path in ["/api/v1/health", "/", "/api/nope"] {
        let (_, headers, _) = get(path).await;
        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'"), "{path}");
        assert!(csp.contains("script-src 'self'"), "{path}");
        assert!(!csp.contains("unsafe-inline"), "{path}");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer", "{path}");
    }
}

#[tokio::test]
async fn spa_routes_serve_html() {
    for path in ["/", "/cameras/123"] {
        let (status, headers, body) = get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("<html"), "{path}");
    }
}

#[tokio::test]
async fn missing_static_file_is_404() {
    let (status, _, _) = get("/assets/nope-123.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn openapi_document_is_served_and_matches_cli_output() {
    let (status, _, body) = get("/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    let served: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let offline = serde_json::to_value(crane::api::openapi()).unwrap();
    assert_eq!(
        served, offline,
        "served and --print-openapi documents differ"
    );
    assert!(served["paths"]["/api/v1/health"]["get"].is_object());
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

#[tokio::test]
async fn swagger_ui_is_served() {
    let app = crane::app(state().await);
    let response = app
        .oneshot(Request::get("/api/docs/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
}

#[tokio::test]
async fn serves_over_tcp_and_shuts_down_gracefully() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crane::serve(listener, state().await, shutdown.clone()));

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
