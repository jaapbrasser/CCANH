//! `/api/v1/cameras/*`: camera registry (read for viewers, write for admins).

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::error::{ApiError, Problem};
use super::extract::{JsonBody, PathParam, QueryParams};
use super::pagination::{Page, PageParams};
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{AdminUser, AuthUser, ClientIp};
use crate::cameras::{
    self, CameraDetail, CameraError, CameraPatch, CameraSettings, CameraSort, CameraState,
    CameraSummary, NewCamera, Scheme, StatusTransition, TlsMode,
};
use crate::events::Event;
use crate::monitoring;
use crate::providers::ProviderTarget;
use crate::users::SortOrder;
use crate::AppState;

/// How long an on-demand identity refresh may take per provider call.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

fn camera_error(err: CameraError) -> ApiError {
    match err {
        CameraError::NotFound => ApiError::NotFound,
        CameraError::Duplicate => {
            ApiError::Conflict("A camera with this host and port already exists.".into())
        }
        CameraError::Invalid(e) => ApiError::BadRequest(e.to_string()),
        CameraError::Database(e) => ApiError::from(e),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CameraListQuery {
    /// 1-based page number (default 1).
    #[param(minimum = 1)]
    pub page: Option<u32>,
    /// Items per page (default 50, max 200).
    #[param(minimum = 1, maximum = 200)]
    pub page_size: Option<u32>,
    /// Case-insensitive substring match on name, host or model.
    pub q: Option<String>,
    #[param(inline)]
    pub status: Option<CameraState>,
    #[param(inline)]
    pub sort: Option<CameraSort>,
    #[param(inline)]
    pub order: Option<SortOrder>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CameraList {
    pub items: Vec<CameraSummary>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct HistoryQuery {
    #[param(minimum = 1)]
    pub page: Option<u32>,
    #[param(minimum = 1, maximum = 200)]
    pub page_size: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusHistory {
    pub items: Vec<StatusTransition>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// List cameras with their current status.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/cameras",
    operation_id = "list_cameras",
    tag = "cameras",
    params(CameraListQuery),
    responses(
        (status = 200, description = "A page of cameras", body = CameraList),
        (status = 400, description = "Invalid query", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn list(
    _user: AuthUser,
    State(state): State<AppState>,
    QueryParams(query): QueryParams<CameraListQuery>,
) -> Result<Json<CameraList>, ApiError> {
    let page = Page::try_from(PageParams {
        page: query.page,
        page_size: query.page_size,
    })?;
    let search = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let (items, total) = cameras::list(
        &state.db,
        cameras::ListParams {
            search,
            state: query.status,
            sort: query.sort.unwrap_or_default(),
            descending: query.order == Some(SortOrder::Desc),
            limit: page.limit(),
            offset: page.offset(),
        },
    )
    .await?;
    Ok(Json(CameraList {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    }))
}

/// Get a camera's details, identity, capabilities and stream profiles.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/cameras/{id}",
    operation_id = "get_camera",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    responses(
        (status = 200, description = "The camera", body = CameraDetail),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
    )
)]
pub async fn get(
    _user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<CameraDetail>, ApiError> {
    cameras::get(&state.db, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// A camera's status transitions, newest first.
///
/// Required role: viewer.
#[utoipa::path(
    get,
    path = "/cameras/{id}/status-history",
    operation_id = "get_camera_status_history",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID"), HistoryQuery),
    responses(
        (status = 200, description = "Status transitions", body = StatusHistory),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
    )
)]
pub async fn status_history(
    _user: AuthUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
    QueryParams(query): QueryParams<HistoryQuery>,
) -> Result<Json<StatusHistory>, ApiError> {
    let page = Page::try_from(PageParams {
        page: query.page,
        page_size: query.page_size,
    })?;
    if cameras::get(&state.db, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let (items, total) = cameras::history(&state.db, id, page.limit(), page.offset()).await?;
    Ok(Json(StatusHistory {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    }))
}

fn warn_if_insecure(camera: &CameraDetail, admin: &AdminUser) {
    if camera.tls_mode == TlsMode::InsecureSkipVerify {
        tracing::warn!(
            camera_id = %camera.summary.id,
            host = %camera.summary.host,
            admin = %admin.username,
            "camera configured with insecure-skip-verify: its TLS certificate will not be checked"
        );
    }
}

fn settings_json(c: &CameraDetail) -> serde_json::Value {
    serde_json::json!({
        "name": c.summary.name,
        "host": c.summary.host,
        "port": c.summary.port,
        "scheme": c.summary.scheme,
        "tls_mode": c.tls_mode,
        "monitoring_enabled": c.summary.monitoring_enabled,
    })
}

/// Register a camera.
///
/// Required role: admin. Credentials are added in Phase 4 (encrypted credential
/// store). `insecure_skip_verify` is logged and audited.
#[utoipa::path(
    post,
    path = "/cameras",
    operation_id = "create_camera",
    tag = "cameras",
    request_body = NewCamera,
    responses(
        (status = 201, description = "Camera registered", body = CameraDetail),
        (status = 400, description = "Invalid settings", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 409, description = "Host and port already registered", body = Problem),
    )
)]
pub async fn create(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<NewCamera>,
) -> Result<(StatusCode, Json<CameraDetail>), ApiError> {
    let settings = CameraSettings::from_new(body).map_err(|e| camera_error(e.into()))?;
    let camera = cameras::create(&state.db, &settings)
        .await
        .map_err(camera_error)?;
    warn_if_insecure(&camera, &admin);
    audit::record(
        &state.db,
        AuditEvent::new(action::CAMERA_CREATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", camera.summary.id)
            .ip(ip)
            .details(settings_json(&camera)),
    )
    .await?;
    state.events.publish(Event::CameraChanged {
        camera: camera.summary.clone(),
    });
    Ok((StatusCode::CREATED, Json(camera)))
}

/// Change a camera's settings.
///
/// Required role: admin. Changing host, port or scheme clears the stored identity
/// and resets status, since it may now be a different device.
#[utoipa::path(
    patch,
    path = "/cameras/{id}",
    operation_id = "update_camera",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    request_body = CameraPatch,
    responses(
        (status = 200, description = "Updated camera", body = CameraDetail),
        (status = 400, description = "Invalid settings", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
        (status = 409, description = "Host and port already registered", body = Problem),
    )
)]
pub async fn update(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<CameraPatch>,
) -> Result<Json<CameraDetail>, ApiError> {
    let (before, after) = cameras::update(&state.db, id, body)
        .await
        .map_err(camera_error)?;
    warn_if_insecure(&after, &admin);
    audit::record(
        &state.db,
        AuditEvent::new(action::CAMERA_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", id)
            .ip(ip)
            .details(serde_json::json!({
                "from": settings_json(&before),
                "to": settings_json(&after),
            })),
    )
    .await?;
    state.events.publish(Event::CameraChanged {
        camera: after.summary.clone(),
    });
    Ok(Json(after))
}

/// Remove a camera, its status and its history.
///
/// Required role: admin.
#[utoipa::path(
    delete,
    path = "/cameras/{id}",
    operation_id = "delete_camera",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
    )
)]
pub async fn delete(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    let camera = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    cameras::delete(&state.db, id).await.map_err(camera_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::CAMERA_DELETE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", id)
            .ip(ip)
            .details(settings_json(&camera)),
    )
    .await?;
    state.events.publish(Event::CameraDeleted { camera_id: id });
    Ok(StatusCode::NO_CONTENT)
}

/// Re-read identity, capabilities and stream profiles from the camera now.
///
/// Required role: admin. Returns 502 with the error category if the camera
/// can't be reached or rejects the request.
#[utoipa::path(
    post,
    path = "/cameras/{id}/refresh",
    operation_id = "refresh_camera",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    responses(
        (status = 200, description = "Refreshed camera", body = CameraDetail),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
        (status = 502, description = "Camera error (see detail)", body = Problem),
    )
)]
pub async fn refresh(
    _admin: AdminUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<CameraDetail>, ApiError> {
    let camera = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let provider = state.providers.create(&ProviderTarget {
        host: camera.summary.host.clone(),
        port: camera.summary.port,
        https: camera.summary.scheme == Scheme::Https,
    });
    match monitoring::refresh_identity(&state.db, &*provider, id, REFRESH_TIMEOUT).await {
        Ok(()) => {}
        Err(monitoring::RefreshError::Provider(e)) => {
            return Err(ApiError::Camera {
                category: e.category(),
                detail: e.to_string(),
            })
        }
        Err(monitoring::RefreshError::Database(e)) => return Err(e.into()),
    }
    let camera = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    state.events.publish(Event::CameraChanged {
        camera: camera.summary.clone(),
    });
    Ok(Json(camera))
}
