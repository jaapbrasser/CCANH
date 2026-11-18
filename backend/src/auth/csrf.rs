//! CSRF protection for state-changing requests (SPEC §14.2, ADR 0004).
//!
//! Two layers:
//! 1. The session cookie is `SameSite=Strict`, so browsers don't attach it to
//!    cross-site requests at all.
//! 2. This middleware rejects any non-safe method (`POST`, `PUT`, `PATCH`,
//!    `DELETE`, …) under `/api/` unless its `Origin` header — or, failing that,
//!    its `Referer` — matches the server's own origin or a configured extra
//!    origin. Requests with neither header are rejected. Browsers always send
//!    `Origin` on cross-origin and on non-GET same-origin `fetch` requests;
//!    non-browser API clients must set it explicitly.

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api::error::ApiError;
use crate::AppState;

fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// `scheme://host[:port]` of an absolute URL, lowercased.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    ))
}

/// Strip a scheme's default port so `https://h:443` equals `https://h`.
fn normalize(origin: &str) -> String {
    let origin = origin.trim_end_matches('/').to_ascii_lowercase();
    origin
        .strip_suffix(":443")
        .filter(|o| o.starts_with("https://"))
        .or_else(|| {
            origin
                .strip_suffix(":80")
                .filter(|o| o.starts_with("http://"))
        })
        .map(str::to_owned)
        .unwrap_or(origin)
}

/// Decide whether a state-changing request's origin is acceptable.
pub fn origin_allowed(headers: &HeaderMap, tls: bool, extra: &[String]) -> bool {
    let presented = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|v| *v != "null")
        .map(str::to_owned)
        .or_else(|| {
            headers
                .get(header::REFERER)
                .and_then(|v| v.to_str().ok())
                .and_then(origin_of)
        });
    let Some(presented) = presented else {
        return false;
    };
    let presented = normalize(&presented);

    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        let scheme = if tls { "https" } else { "http" };
        if presented == normalize(&format!("{scheme}://{host}")) {
            return true;
        }
    }
    extra.iter().any(|o| normalize(o) == presented)
}

pub async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if !is_safe(request.method())
        && request.uri().path().starts_with("/api/")
        && !origin_allowed(
            request.headers(),
            state.config.tls_enabled(),
            &state.config.server.allowed_origins,
        )
    {
        tracing::warn!(
            method = %request.method(),
            path = request.uri().path(),
            origin = ?request.headers().get(header::ORIGIN),
            "rejected state-changing request with foreign or missing Origin"
        );
        return ApiError::CsrfRejected.into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn same_origin_is_allowed() {
        let h = headers(&[
            ("host", "crane.local:8080"),
            ("origin", "http://crane.local:8080"),
        ]);
        assert!(origin_allowed(&h, false, &[]));
    }

    #[test]
    fn scheme_must_match_tls() {
        let h = headers(&[("host", "crane.local"), ("origin", "http://crane.local")]);
        assert!(!origin_allowed(&h, true, &[]));
        let h = headers(&[("host", "crane.local"), ("origin", "https://crane.local")]);
        assert!(origin_allowed(&h, true, &[]));
    }

    #[test]
    fn default_ports_are_normalized() {
        let h = headers(&[
            ("host", "crane.local:443"),
            ("origin", "https://crane.local"),
        ]);
        assert!(origin_allowed(&h, true, &[]));
    }

    #[test]
    fn foreign_origin_is_rejected() {
        let h = headers(&[
            ("host", "crane.local:8080"),
            ("origin", "http://evil.example"),
        ]);
        assert!(!origin_allowed(&h, false, &[]));
    }

    #[test]
    fn missing_or_null_origin_is_rejected() {
        assert!(!origin_allowed(
            &headers(&[("host", "crane.local")]),
            false,
            &[]
        ));
        let h = headers(&[("host", "crane.local"), ("origin", "null")]);
        assert!(!origin_allowed(&h, false, &[]));
    }

    #[test]
    fn referer_is_a_fallback() {
        let h = headers(&[
            ("host", "crane.local:8080"),
            ("referer", "http://crane.local:8080/users?x=1"),
        ]);
        assert!(origin_allowed(&h, false, &[]));
        let h = headers(&[
            ("host", "crane.local:8080"),
            ("referer", "http://evil.example/"),
        ]);
        assert!(!origin_allowed(&h, false, &[]));
    }

    #[test]
    fn configured_extra_origins_are_allowed() {
        let h = headers(&[
            ("host", "10.0.0.5:8443"),
            ("origin", "https://crane.site-a.local:8443"),
        ]);
        assert!(origin_allowed(
            &h,
            true,
            &["https://crane.site-a.local:8443".into()]
        ));
    }

    #[test]
    fn host_lookalikes_are_rejected() {
        let h = headers(&[
            ("host", "crane.local"),
            ("origin", "http://crane.local.evil.example"),
        ]);
        assert!(!origin_allowed(&h, false, &[]));
        let h = headers(&[
            ("host", "crane.local"),
            ("referer", "http://crane.local@evil.example/"),
        ]);
        assert!(!origin_allowed(&h, false, &[]));
    }
}
