//! `/api/v1/setup`: first-run creation of the initial admin (SPEC §14.2).

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::Json;
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::auth::{password_policy_error, session_cookie, Me};
use super::error::{ApiError, Problem};
use super::extract::JsonBody;
use super::users::user_error;
use crate::audit::{self, action, Event, Outcome};
use crate::auth::session::{self, Timeouts};
use crate::auth::{password, ClientIp};
use crate::secret::{ExposeSecret, SecretString};
use crate::users;
use crate::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatus {
    /// True when no users exist and the setup page should be shown.
    pub setup_required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    /// The one-time token printed to the server console and log at startup.
    #[schema(value_type = String, format = Password)]
    pub token: SecretString,
    pub username: String,
    #[schema(value_type = String, format = Password, min_length = 12)]
    pub password: SecretString,
}

/// Whether first-run setup is needed.
///
/// Required role: none (public).
#[utoipa::path(
    get,
    path = "/setup",
    operation_id = "get_setup_status",
    tag = "setup",
    responses((status = 200, description = "Setup status", body = SetupStatus))
)]
pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, ApiError> {
    let setup_required = users::count(&state.db).await? == 0;
    Ok(Json(SetupStatus { setup_required }))
}

/// Create the first admin.
///
/// Required role: none (public), but needs the one-time setup token printed at
/// startup. Only works while no users exist. Signs the new admin in.
#[utoipa::path(
    post,
    path = "/setup",
    operation_id = "complete_setup",
    tag = "setup",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "Admin created and signed in", body = Me),
        (status = 400, description = "Invalid username or password", body = Problem),
        (status = 401, description = "Wrong or expired setup token", body = Problem),
        (status = 409, description = "Setup already completed", body = Problem),
    )
)]
pub async fn complete(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    JsonBody(body): JsonBody<SetupRequest>,
) -> Result<(StatusCode, CookieJar, Json<Me>), ApiError> {
    if users::count(&state.db).await? > 0 {
        return Err(ApiError::Conflict(
            "Setup has already been completed.".into(),
        ));
    }
    // Rate limit token guessing like logins (the key is a fixed pseudo-username).
    if let Err(wait) = state.login_limiter.check("\u{0}setup", ip) {
        return Err(ApiError::TooManyRequests {
            retry_after_secs: wait.as_secs().max(1),
        });
    }
    if !state.setup.verify(body.token.expose_secret()) {
        state.login_limiter.record_failure("\u{0}setup", ip);
        audit::record_best_effort(
            &state.db,
            Event::new(action::SETUP_COMPLETE, Outcome::Failure)
                .actor(None, &body.username)
                .ip(ip)
                .details(serde_json::json!({ "reason": "bad_token" })),
        )
        .await;
        return Err(ApiError::Unauthorized);
    }

    password::validate(&body.password).map_err(password_policy_error)?;
    users::validate_username(&body.username).map_err(user_error)?;
    let hash = password::hash_blocking(body.password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let user = users::create_first_admin(&state.db, &body.username, &hash)
        .await
        .map_err(user_error)?
        .ok_or_else(|| ApiError::Conflict("Setup has already been completed.".into()))?;
    state.setup.clear();

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let token = session::create(
        &state.db,
        user.id,
        ip,
        user_agent,
        Timeouts::from(&state.config.auth),
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    audit::record(
        &state.db,
        Event::new(action::SETUP_COMPLETE, Outcome::Success)
            .actor(Some(user.id), &user.username)
            .target("user", user.id)
            .ip(ip),
    )
    .await?;
    tracing::info!(username = %user.username, "first admin created via setup");

    Ok((
        StatusCode::CREATED,
        jar.add(session_cookie(&state, token)),
        Json(Me {
            id: user.id,
            username: user.username,
            role: user.role,
        }),
    ))
}
