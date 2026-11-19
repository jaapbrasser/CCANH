//! Per-user dashboards (SPEC §8). Every route only ever sees the caller's
//! own dashboards; someone else's id is a 404.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam};
use crate::auth::AuthUser;
use crate::dashboards::{self, Dashboard, DashboardError, DashboardPatch, NewDashboard};
use crate::AppState;

fn dashboard_error(e: DashboardError) -> ApiError {
    match e {
        DashboardError::NotFound => ApiError::NotFound,
        DashboardError::Invalid(m) => ApiError::BadRequest(m),
        DashboardError::Database(e) => ApiError::from(e),
    }
}

/// List the signed-in user's dashboards, oldest first.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/dashboards",
    operation_id = "list_dashboards",
    tag = "dashboards",
    responses(
        (status = 200, description = "The caller's dashboards", body = Vec<Dashboard>),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn list(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Dashboard>>, ApiError> {
    Ok(Json(dashboards::list(&state.db, user.id).await?))
}

/// Create a dashboard. The user's first dashboard becomes their default.
///
/// Required role: viewer.
#[utoipa::path(
    post,
    path = "/dashboards",
    operation_id = "create_dashboard",
    tag = "dashboards",
    request_body = NewDashboard,
    responses(
        (status = 201, description = "Created", body = Dashboard),
        (status = 400, description = "Invalid name, grid or tiles", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    JsonBody(body): JsonBody<NewDashboard>,
) -> Result<(StatusCode, Json<Dashboard>), ApiError> {
    let d = dashboards::create(&state.db, user.id, &body)
        .await
        .map_err(dashboard_error)?;
    Ok((StatusCode::CREATED, Json(d)))
}

/// Get one of the signed-in user's dashboards.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/dashboards/{id}",
    operation_id = "get_dashboard",
    tag = "dashboards",
    params(("id" = Uuid, Path, description = "Dashboard ID")),
    responses(
        (status = 200, description = "The dashboard", body = Dashboard),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such dashboard of yours", body = Problem),
    )
)]
pub async fn get(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<Dashboard>, ApiError> {
    dashboards::get(&state.db, user.id, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Rename, resize, rearrange or make default.
///
/// Required role: viewer. `tiles` replaces all tiles; each camera at most
/// once, positions inside the grid.
#[utoipa::path(
    patch,
    path = "/dashboards/{id}",
    operation_id = "update_dashboard",
    tag = "dashboards",
    params(("id" = Uuid, Path, description = "Dashboard ID")),
    request_body = DashboardPatch,
    responses(
        (status = 200, description = "Updated", body = Dashboard),
        (status = 400, description = "Invalid name, grid or tiles", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such dashboard of yours", body = Problem),
    )
)]
pub async fn update(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<DashboardPatch>,
) -> Result<Json<Dashboard>, ApiError> {
    let d = dashboards::update(&state.db, user.id, id, &body)
        .await
        .map_err(dashboard_error)?;
    Ok(Json(d))
}

/// Delete a dashboard. If it was the default, the oldest remaining one
/// becomes the default.
///
/// Required role: viewer.
#[utoipa::path(
    delete,
    path = "/dashboards/{id}",
    operation_id = "delete_dashboard",
    tag = "dashboards",
    params(("id" = Uuid, Path, description = "Dashboard ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such dashboard of yours", body = Problem),
    )
)]
pub async fn delete(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    dashboards::delete(&state.db, user.id, id)
        .await
        .map_err(dashboard_error)?;
    Ok(StatusCode::NO_CONTENT)
}
