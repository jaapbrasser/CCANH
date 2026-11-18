//! Shared helpers for integration tests: an in-process app with an in-memory
//! database and a small request builder that behaves like a same-origin browser.

#![allow(dead_code)] // each test binary uses a different subset

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crane::auth::{password, Role};
use crane::config::Config;
use crane::secret::SecretString;
use crane::{storage, users, AppState};

pub const ORIGIN: &str = "http://localhost";
pub const PASSWORD: &str = "correct horse battery staple";

pub struct TestApp {
    pub state: AppState,
    pub router: Router,
}

pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// `crane_session=<token>` from Set-Cookie, if any.
    pub fn session_cookie(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("crane_session="))
            .map(|v| v.split(';').next().unwrap_or_default().to_owned())
    }

    pub fn set_cookie_header(&self) -> String {
        self.headers
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default()
    }
}

pub struct Req {
    method: Method,
    path: String,
    body: Option<Value>,
    cookie: Option<String>,
    origin: Option<&'static str>,
}

pub fn req(method: Method, path: &str) -> Req {
    Req {
        method,
        path: path.to_owned(),
        body: None,
        cookie: None,
        origin: Some(ORIGIN),
    }
}

impl Req {
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    pub fn cookie(mut self, cookie: &str) -> Self {
        self.cookie = Some(cookie.to_owned());
        self
    }

    pub fn origin(mut self, origin: Option<&'static str>) -> Self {
        self.origin = origin;
        self
    }
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(Config::default()).await
    }

    pub async fn with_config(config: Config) -> Self {
        let db = storage::connect_in_memory().await.unwrap();
        let store = std::sync::Arc::new(crane::credentials::AesGcmStore::ephemeral());
        let state = AppState::new(config, db, store);
        let router = crane::app(state.clone());
        Self { state, router }
    }

    pub async fn send(&self, r: Req) -> Response {
        let mut builder = Request::builder()
            .method(r.method)
            .uri(&r.path)
            .header(header::HOST, "localhost");
        if let Some(origin) = r.origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(cookie) = &r.cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        let body = match r.body {
            Some(v) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(builder.body(body).unwrap())
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
        Response {
            status,
            headers,
            body,
        }
    }

    /// Create a user directly in the database with [`PASSWORD`].
    pub async fn user(&self, username: &str, role: Role) -> users::User {
        let hash = password::hash(&SecretString::from(PASSWORD.to_owned())).unwrap();
        users::create(&self.state.db, username, &hash, role)
            .await
            .unwrap()
    }

    /// Log in and return the `crane_session=...` cookie pair.
    pub async fn login(&self, username: &str) -> String {
        let res = self
            .send(
                req(Method::POST, "/api/v1/auth/login").json(serde_json::json!({
                    "username": username,
                    "password": PASSWORD,
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "login failed: {}", res.text());
        res.session_cookie().expect("no session cookie")
    }

    /// Create a user and return their session cookie.
    pub async fn signed_in(&self, username: &str, role: Role) -> (users::User, String) {
        let user = self.user(username, role).await;
        let cookie = self.login(username).await;
        (user, cookie)
    }
}
