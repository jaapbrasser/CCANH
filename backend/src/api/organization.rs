//! Groups and favorites (SPEC §9).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam, QueryParams};
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{AdminUser, AuthUser, ClientIp};
use crate::events::{Event, OrganizationKind};
use crate::organization::{
    favorites, groups, DeleteGroupQuery, Group, GroupPatch, NewGroup, OrgError,
};
use crate::AppState;

pub(super) fn org_error(e: OrgError) -> ApiError {
    match e {
        OrgError::NotFound => ApiError::NotFound,
        OrgError::Duplicate(m) => ApiError::Conflict(m),
        OrgError::Invalid(m) => ApiError::BadRequest(m),
        OrgError::Database(e) => ApiError::from(e),
    }
}

/// Cameras whose memberships changed get a fresh `camera_changed` event.
pub(super) async fn announce_cameras(state: &AppState, ids: &[Uuid]) -> Result<(), ApiError> {
    for id in ids {
        if let Some(c) = crate::cameras::get(&state.db, *id).await? {
            state
                .events
                .publish(Event::CameraChanged { camera: c.summary });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// List all groups (flat; build the tree from `parent_id`).
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/groups",
    operation_id = "list_groups",
    tag = "organization",
    responses(
        (status = 200, description = "All groups, by name", body = Vec<Group>),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn list_groups(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Group>>, ApiError> {
    Ok(Json(groups::list(&state.db).await?))
}

/// Create a group, optionally under a parent.
///
/// Required role: admin. Sibling names must be unique (ignoring case).
#[utoipa::path(
    post,
    path = "/groups",
    operation_id = "create_group",
    tag = "organization",
    request_body = NewGroup,
    responses(
        (status = 201, description = "Created", body = Group),
        (status = 400, description = "Invalid name or parent", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 409, description = "Name taken at that level", body = Problem),
    )
)]
pub async fn create_group(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<NewGroup>,
) -> Result<(StatusCode, Json<Group>), ApiError> {
    let group = groups::create(&state.db, &body).await.map_err(org_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::GROUP_CREATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("group", group.id)
            .ip(ip)
            .details(serde_json::json!({ "name": group.name, "parent_id": group.parent_id })),
    )
    .await?;
    state.events.publish(Event::OrganizationChanged {
        kind: OrganizationKind::Groups,
    });
    Ok((StatusCode::CREATED, Json(group)))
}

/// Rename and/or move a group.
///
/// Required role: admin. `parent_id: null` moves it to the top level. Moving
/// a group into itself or one of its subgroups is refused.
#[utoipa::path(
    patch,
    path = "/groups/{id}",
    operation_id = "update_group",
    tag = "organization",
    params(("id" = Uuid, Path, description = "Group ID")),
    request_body = GroupPatch,
    responses(
        (status = 200, description = "Updated", body = Group),
        (status = 400, description = "Invalid name, parent or move", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such group", body = Problem),
        (status = 409, description = "Name taken at that level", body = Problem),
    )
)]
pub async fn update_group(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<GroupPatch>,
) -> Result<Json<Group>, ApiError> {
    let (before, after) = groups::update(&state.db, id, &body)
        .await
        .map_err(org_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::GROUP_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("group", id)
            .ip(ip)
            .details(serde_json::json!({
                "from": { "name": before.name, "parent_id": before.parent_id },
                "to": { "name": after.name, "parent_id": after.parent_id },
            })),
    )
    .await?;
    state.events.publish(Event::OrganizationChanged {
        kind: OrganizationKind::Groups,
    });
    if before.name != after.name {
        announce_cameras(&state, &groups::direct_members(&state.db, id).await?).await?;
    }
    Ok(Json(after))
}

/// Delete a group.
///
/// Required role: admin. `mode=move_up` moves subgroups and cameras to the
/// parent group (for a top-level group: subgroups become top-level, cameras
/// leave it); `mode=detach` makes subgroups top-level and removes the cameras
/// from the group. Cameras themselves are never deleted.
#[utoipa::path(
    delete,
    path = "/groups/{id}",
    operation_id = "delete_group",
    tag = "organization",
    params(("id" = Uuid, Path, description = "Group ID"), DeleteGroupQuery),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "Missing or invalid mode", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such group", body = Problem),
        (status = 409, description = "A subgroup's name clashes where it would move", body = Problem),
    )
)]
pub async fn delete_group(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    QueryParams(q): QueryParams<DeleteGroupQuery>,
) -> Result<StatusCode, ApiError> {
    // Direct members only: cameras in subgroups keep their memberships.
    let direct = groups::direct_members(&state.db, id).await?;
    let group = groups::delete(&state.db, id, q.mode)
        .await
        .map_err(org_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::GROUP_DELETE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("group", id)
            .ip(ip)
            .details(serde_json::json!({
                "name": group.name, "mode": q.mode, "cameras": direct.len(),
            })),
    )
    .await?;
    state.events.publish(Event::OrganizationChanged {
        kind: OrganizationKind::Groups,
    });
    announce_cameras(&state, &direct).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Favorites (per user)
// ---------------------------------------------------------------------------

/// The signed-in user's favorite camera ids, most recent first.
///
/// Required role: viewer. Only ever the caller's own favorites.
#[utoipa::path(
    get,
    path = "/favorites",
    operation_id = "list_favorites",
    tag = "organization",
    responses(
        (status = 200, description = "Favorite camera ids", body = Vec<Uuid>),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn list_favorites(
    user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Uuid>>, ApiError> {
    Ok(Json(favorites::list(&state.db, user.id).await?))
}

/// Mark a camera as a favorite of the signed-in user (idempotent).
///
/// Required role: viewer.
#[utoipa::path(
    put,
    path = "/favorites/{camera_id}",
    operation_id = "add_favorite",
    tag = "organization",
    params(("camera_id" = Uuid, Path, description = "Camera ID")),
    responses(
        (status = 204, description = "Added"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
    )
)]
pub async fn add_favorite(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(camera_id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    favorites::add(&state.db, user.id, camera_id)
        .await
        .map_err(org_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a camera from the signed-in user's favorites (idempotent).
///
/// Required role: viewer.
#[utoipa::path(
    delete,
    path = "/favorites/{camera_id}",
    operation_id = "remove_favorite",
    tag = "organization",
    params(("camera_id" = Uuid, Path, description = "Camera ID")),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn remove_favorite(
    user: AuthUser,
    State(state): State<AppState>,
    PathParam(camera_id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    favorites::remove(&state.db, user.id, camera_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
