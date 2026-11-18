//! RFC 9457 `application/problem+json` error responses.
//!
//! Every API error is mapped to an [`ApiError`]. The response carries a stable
//! machine-readable `type` and `code`, a human-readable `title`/`detail`, and never
//! any internal error text: internal causes are logged, not returned.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

pub const PROBLEM_JSON: &str = "application/problem+json";

/// Problem details body (RFC 9457).
#[derive(Debug, Serialize, ToSchema)]
pub struct Problem {
    /// URI identifying the problem type, e.g. `urn:crane:problem:not_found`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Short summary of the problem type.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Explanation specific to this occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Stable error code, the last segment of `type`.
    pub code: String,
}

/// Errors returned by HTTP handlers.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    /// No valid session.
    #[error("unauthorized")]
    Unauthorized,
    /// Login failed. Deliberately vague: never says whether the user exists.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// Authenticated but lacking the required role.
    #[error("forbidden")]
    Forbidden,
    /// A state-changing request failed the Origin/Referer check.
    #[error("cross-site request rejected")]
    CsrfRejected,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("too many requests")]
    TooManyRequests { retry_after_secs: u64 },
    #[error("service unavailable: {0}")]
    Unavailable(String),
    /// Unexpected failure. The message is logged but never sent to the client.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str, Option<String>) {
        match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "Not Found", None),
            ApiError::BadRequest(detail) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                "Bad Request",
                Some(detail.clone()),
            ),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Unauthorized",
                Some("Sign in to continue.".into()),
            ),
            ApiError::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid Credentials",
                Some("The username or password is incorrect.".into()),
            ),
            ApiError::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "Forbidden",
                Some("You don't have permission to do this.".into()),
            ),
            ApiError::CsrfRejected => (
                StatusCode::FORBIDDEN,
                "csrf_rejected",
                "Cross-Site Request Rejected",
                Some("The request's Origin is not allowed.".into()),
            ),
            ApiError::Conflict(detail) => (
                StatusCode::CONFLICT,
                "conflict",
                "Conflict",
                Some(detail.clone()),
            ),
            ApiError::TooManyRequests { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too Many Requests",
                Some("Too many attempts. Try again later.".into()),
            ),
            ApiError::Unavailable(detail) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Service Unavailable",
                Some(detail.clone()),
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Internal Server Error",
                None,
            ),
        }
    }

    pub fn to_problem(&self) -> Problem {
        let (status, code, title, detail) = self.parts();
        Problem {
            kind: format!("urn:crane:problem:{code}"),
            title: title.to_owned(),
            status: status.as_u16(),
            detail,
            code: code.to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let ApiError::Internal(message) = &self {
            tracing::error!(error = %message, "internal API error");
        }
        let problem = self.to_problem();
        let status =
            StatusCode::from_u16(problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&problem).unwrap_or_default();
        let mut response = (status, [(header::CONTENT_TYPE, PROBLEM_JSON)], body).into_response();
        if let ApiError::TooManyRequests { retry_after_secs } = self {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, retry_after_secs.max(1).into());
        }
        response
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::Internal(format!("database: {err}"))
    }
}

impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        ApiError::BadRequest(rejection.body_text())
    }
}

impl From<axum::extract::rejection::PathRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::PathRejection) -> Self {
        ApiError::BadRequest(rejection.body_text())
    }
}

impl From<axum::extract::rejection::QueryRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        ApiError::BadRequest(rejection.body_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_do_not_leak_details() {
        let problem = ApiError::Internal("password=hunter2 at db.rs:12".into()).to_problem();
        let json = serde_json::to_string(&problem).unwrap();
        assert!(!json.contains("hunter2"));
        assert_eq!(problem.status, 500);
        assert_eq!(problem.code, "internal");
    }

    #[test]
    fn problem_has_stable_type() {
        let problem = ApiError::NotFound.to_problem();
        assert_eq!(problem.kind, "urn:crane:problem:not_found");
        assert_eq!(problem.status, 404);
    }
}
