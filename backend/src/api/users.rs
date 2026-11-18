//! `/api/v1/users/*`: user administration (admin only).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::auth::password_policy_error;
use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam, QueryParams};
use super::pagination::{Page, PageParams};
use crate::audit::{self, action, Event, Outcome};
use crate::auth::{password, session, AdminUser, ClientIp, Role};
use crate::secret::SecretString;
use crate::users::{self, SortOrder, User, UserError, UserSort};
use crate::AppState;

pub(crate) fn user_error(err: UserError) -> ApiError {
    match err {
        UserError::UsernameTaken => ApiError::Conflict("That username is already taken.".into()),
        UserError::NotFound => ApiError::NotFound,
        UserError::LastAdmin => ApiError::Conflict(
            "This would leave no enabled admin. Promote or enable another admin first.".into(),
        ),
        UserError::InvalidUsername(why) => ApiError::BadRequest(format!("Username {why}.")),
        UserError::Database(e) => ApiError::from(e),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UserListQuery {
    /// 1-based page number (default 1).
    #[param(minimum = 1)]
    pub page: Option<u32>,
    /// Items per page (default 50, max 200).
    #[param(minimum = 1, maximum = 200)]
    pub page_size: Option<u32>,
    /// Case-insensitive substring match on username.
    pub q: Option<String>,
    #[param(inline)]
    pub sort: Option<UserSort>,
    #[param(inline)]
    pub order: Option<SortOrder>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserList {
    pub items: Vec<User>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    #[schema(value_type = String, format = Password, min_length = 12)]
    pub password: SecretString,
    pub role: Role,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub role: Option<Role>,
    pub disabled: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    #[schema(value_type = String, format = Password, min_length = 12)]
    pub new_password: SecretString,
}

/// List users.
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/users",
    operation_id = "list_users",
    tag = "users",
    params(UserListQuery),
    responses(
        (status = 200, description = "A page of users", body = UserList),
        (status = 400, description = "Invalid query", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    QueryParams(query): QueryParams<UserListQuery>,
) -> Result<Json<UserList>, ApiError> {
    let page = Page::try_from(PageParams {
        page: query.page,
        page_size: query.page_size,
    })?;
    let search = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let (items, total) = users::list(
        &state.db,
        users::ListParams {
            search,
            sort: query.sort.unwrap_or_default(),
            order: query.order.unwrap_or_default(),
            limit: page.limit(),
            offset: page.offset(),
        },
    )
    .await?;
    Ok(Json(UserList {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    }))
}

/// Create a user.
///
/// Required role: admin.
#[utoipa::path(
    post,
    path = "/users",
    operation_id = "create_user",
    tag = "users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = User),
        (status = 400, description = "Invalid username or password", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 409, description = "Username taken", body = Problem),
    )
)]
pub async fn create(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<CreateUserRequest>,
) -> Result<(StatusCode, Json<User>), ApiError> {
    users::validate_username(&body.username).map_err(user_error)?;
    password::validate(&body.password).map_err(password_policy_error)?;
    let hash = password::hash_blocking(body.password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let user = users::create(&state.db, &body.username, &hash, body.role)
        .await
        .map_err(user_error)?;
    audit::record(
        &state.db,
        Event::new(action::USER_CREATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("user", user.id)
            .ip(ip)
            .details(serde_json::json!({ "username": user.username, "role": user.role })),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(user)))
}

/// Get a user.
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/users/{id}",
    operation_id = "get_user",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    responses(
        (status = 200, description = "The user", body = User),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<User>, ApiError> {
    users::get(&state.db, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Change a user's role or disabled flag.
///
/// Required role: admin. The last enabled admin can't be demoted or disabled.
/// Disabling a user ends all their sessions.
#[utoipa::path(
    patch,
    path = "/users/{id}",
    operation_id = "update_user",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "Updated user", body = User),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "Would leave no enabled admin", body = Problem),
    )
)]
pub async fn update(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<UpdateUserRequest>,
) -> Result<Json<User>, ApiError> {
    let before = users::get(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    let user = users::update(&state.db, id, body.role, body.disabled)
        .await
        .map_err(user_error)?;
    if user.disabled && !before.disabled {
        session::delete_for_user(&state.db, id, None).await?;
    }
    audit::record(
        &state.db,
        Event::new(action::USER_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("user", id)
            .ip(ip)
            .details(serde_json::json!({
                "username": user.username,
                "role": { "from": before.role, "to": user.role },
                "disabled": { "from": before.disabled, "to": user.disabled },
            })),
    )
    .await?;
    Ok(Json(user))
}

/// Delete a user.
///
/// Required role: admin. The last enabled admin can't be deleted, and admins
/// can't delete their own account.
#[utoipa::path(
    delete,
    path = "/users/{id}",
    operation_id = "delete_user",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "Own account, or the last enabled admin", body = Problem),
    )
)]
pub async fn delete(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    if id == admin.id {
        return Err(ApiError::Conflict(
            "You can't delete your own account.".into(),
        ));
    }
    let user = users::get(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    users::delete(&state.db, id).await.map_err(user_error)?;
    audit::record(
        &state.db,
        Event::new(action::USER_DELETE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("user", id)
            .ip(ip)
            .details(serde_json::json!({ "username": user.username })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reset a user's password.
///
/// Required role: admin. Ends all of that user's sessions.
#[utoipa::path(
    post,
    path = "/users/{id}/password",
    operation_id = "reset_user_password",
    tag = "users",
    params(("id" = Uuid, Path, description = "User ID")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "Password reset"),
        (status = 400, description = "Password violates the policy", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn reset_password(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    password::validate(&body.new_password).map_err(password_policy_error)?;
    let user = users::get(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    let hash = password::hash_blocking(body.new_password)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    users::set_password(&state.db, id, &hash)
        .await
        .map_err(user_error)?;
    // An admin resetting their own password keeps their current session.
    let keep = (id == admin.id).then_some(admin.session_hash.as_str());
    session::delete_for_user(&state.db, id, keep).await?;
    audit::record(
        &state.db,
        Event::new(action::USER_PASSWORD_RESET, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("user", id)
            .ip(ip)
            .details(serde_json::json!({ "username": user.username })),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
