//! `/api/v1/auth/*`: login, logout, current user, change own password.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::JsonBody;
use crate::audit::{self, action, Event, Outcome};
use crate::auth::session::{self, Timeouts, COOKIE_NAME};
use crate::auth::{password, AuthUser, ClientIp, Role};
use crate::secret::SecretString;
use crate::users;
use crate::AppState;

/// The signed-in user.
#[derive(Debug, Serialize, ToSchema)]
pub struct Me {
    pub id: Uuid,
    pub username: String,
    pub role: Role,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    #[schema(value_type = String, format = Password)]
    pub password: SecretString,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    #[schema(value_type = String, format = Password)]
    pub current_password: SecretString,
    #[schema(value_type = String, format = Password, min_length = 12)]
    pub new_password: SecretString,
}

pub(crate) fn session_cookie(state: &AppState, token: String) -> Cookie<'static> {
    let max_age = Timeouts::from(&state.config.auth).absolute;
    Cookie::build((COOKIE_NAME, token))
        .path("/")
        .http_only(true)
        .secure(state.config.tls_enabled())
        .same_site(SameSite::Strict)
        .max_age(time_duration(max_age))
        .build()
}

fn removal_cookie(state: &AppState) -> Cookie<'static> {
    let mut cookie = session_cookie(state, String::new());
    cookie.make_removal();
    cookie
}

fn time_duration(d: std::time::Duration) -> time::Duration {
    time::Duration::seconds(i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

pub(crate) fn password_policy_error(err: password::PasswordError) -> ApiError {
    match err {
        password::PasswordError::TooShort | password::PasswordError::TooLong => {
            ApiError::BadRequest(err.to_string())
        }
        other => ApiError::Internal(other.to_string()),
    }
}

/// Sign in.
///
/// Required role: none (public). On success sets the `crane_session` cookie
/// (`HttpOnly`, `SameSite=Strict`, `Secure` under TLS). Failed attempts are rate
/// limited per username and per IP with exponential delay and are audited.
#[utoipa::path(
    post,
    path = "/auth/login",
    operation_id = "login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in", body = Me,
            headers(("set-cookie" = String, description = "Session cookie"))),
        (status = 401, description = "Wrong username or password", body = Problem),
        (status = 429, description = "Too many failed attempts", body = Problem,
            headers(("retry-after" = u64, description = "Seconds to wait"))),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    JsonBody(body): JsonBody<LoginRequest>,
) -> Result<(CookieJar, Json<Me>), ApiError> {
    let username = body.username.trim();
    if let Err(wait) = state.login_limiter.check(username, ip) {
        tracing::warn!(?ip, "login rate limited");
        return Err(ApiError::TooManyRequests {
            retry_after_secs: wait.as_secs().max(1),
        });
    }

    let creds = users::find_credentials(&state.db, username).await?;
    let stored = creds.as_ref().map(|c| c.password_hash.clone());
    let valid = password::verify_blocking(body.password, stored)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let failure_reason = match &creds {
        None => Some("unknown_user"),
        Some(_) if !valid => Some("bad_password"),
        Some(c) if c.user.disabled => Some("disabled"),
        Some(_) => None,
    };

    if let Some(reason) = failure_reason {
        state.login_limiter.record_failure(username, ip);
        audit::record_best_effort(
            &state.db,
            Event::new(action::LOGIN, Outcome::Failure)
                .actor(creds.as_ref().map(|c| c.user.id), username)
                .ip(ip)
                .details(serde_json::json!({ "reason": reason })),
        )
        .await;
        return Err(ApiError::InvalidCredentials);
    }

    // `failure_reason` is None only when `creds` is Some.
    let user = creds.ok_or(ApiError::InvalidCredentials)?.user;
    state.login_limiter.record_success(username);
    let token = session::create(
        &state.db,
        user.id,
        ip,
        user_agent(&headers),
        Timeouts::from(&state.config.auth),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    audit::record(
        &state.db,
        Event::new(action::LOGIN, Outcome::Success)
            .actor(Some(user.id), &user.username)
            .ip(ip),
    )
    .await?;

    Ok((
        jar.add(session_cookie(&state, token)),
        Json(Me {
            id: user.id,
            username: user.username,
            role: user.role,
        }),
    ))
}

/// Sign out.
///
/// Required role: viewer. Ends the current session and clears the cookie.
#[utoipa::path(
    post,
    path = "/auth/logout",
    operation_id = "logout",
    tag = "auth",
    responses(
        (status = 204, description = "Signed out"),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn logout(
    user: AuthUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
) -> Result<(StatusCode, CookieJar), ApiError> {
    session::delete(&state.db, &user.session_hash).await?;
    audit::record(
        &state.db,
        Event::new(action::LOGOUT, Outcome::Success)
            .actor(Some(user.id), &user.username)
            .ip(ip),
    )
    .await?;
    Ok((StatusCode::NO_CONTENT, jar.add(removal_cookie(&state))))
}

/// Current user.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/auth/me",
    operation_id = "get_me",
    tag = "auth",
    responses(
        (status = 200, description = "The signed-in user", body = Me),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn me(user: AuthUser) -> Json<Me> {
    Json(Me {
        id: user.id,
        username: user.username,
        role: user.role,
    })
}

/// Change own password.
///
/// Required role: viewer. Requires the current password. Ends all of the user's
/// other sessions; the current one stays signed in.
#[utoipa::path(
    post,
    path = "/auth/password",
    operation_id = "change_own_password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed"),
        (status = 400, description = "New password violates the policy", body = Problem),
        (status = 401, description = "Not signed in, or current password wrong", body = Problem),
        (status = 429, description = "Too many failed attempts", body = Problem),
    )
)]
pub async fn change_password(
    user: AuthUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    password::validate(&body.new_password).map_err(password_policy_error)?;
    if let Err(wait) = state.login_limiter.check(&user.username, ip) {
        return Err(ApiError::TooManyRequests {
            retry_after_secs: wait.as_secs().max(1),
        });
    }

    let stored = users::password_hash(&state.db, user.id).await?;
    let valid = password::verify_blocking(body.current_password, stored)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !valid {
        state.login_limiter.record_failure(&user.username, ip);
        audit::record_best_effort(
            &state.db,
            Event::new(action::PASSWORD_CHANGE, Outcome::Failure)
                .actor(Some(user.id), &user.username)
                .target("user", user.id)
                .ip(ip)
                .details(serde_json::json!({ "reason": "bad_current_password" })),
        )
        .await;
        return Err(ApiError::InvalidCredentials);
    }

    let hash = password::hash_blocking(body.new_password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    users::set_password(&state.db, user.id, &hash)
        .await
        .map_err(super::users::user_error)?;
    let ended = session::delete_for_user(&state.db, user.id, Some(&user.session_hash)).await?;

    audit::record(
        &state.db,
        Event::new(action::PASSWORD_CHANGE, Outcome::Success)
            .actor(Some(user.id), &user.username)
            .target("user", user.id)
            .ip(ip)
            .details(serde_json::json!({ "other_sessions_ended": ended })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
