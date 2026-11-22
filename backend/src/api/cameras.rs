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
    CameraSummary, CredentialInput, NewCamera, Scheme, StatusTransition, StreamProfileOverrides,
    TlsMode,
};
use crate::events::Event;
use crate::monitoring;
use crate::organization::groups;
use crate::users::SortOrder;
use crate::AppState;

/// How long an on-demand identity refresh may take per provider call.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn camera_error(err: CameraError) -> ApiError {
    match err {
        CameraError::NotFound => ApiError::NotFound,
        CameraError::Duplicate => {
            ApiError::Conflict("A camera with this host and port already exists.".into())
        }
        CameraError::Invalid(e) => ApiError::BadRequest(e.to_string()),
        CameraError::UnknownCredentialSet => {
            ApiError::BadRequest("The selected credential set does not exist.".into())
        }
        CameraError::Credentials(e) => super::credentials::credential_error(e),
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
    /// Only cameras in this group or any of its subgroups.
    pub group_id: Option<Uuid>,
    /// `true`: only the signed-in user's favorites.
    pub favorites: Option<bool>,
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

/// Status counts of the monitored cameras (cameras with monitoring off are
/// left out).
///
/// Required role: viewer. The UI refreshes it on `camera_status` events.
#[utoipa::path(
    get,
    path = "/cameras/summary",
    operation_id = "camera_status_summary",
    tag = "cameras",
    responses(
        (status = 200, description = "Counts per state", body = cameras::StatusSummary),
        (status = 401, description = "Not signed in", body = Problem),
    )
)]
pub async fn summary(
    _user: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<cameras::StatusSummary>, ApiError> {
    Ok(Json(cameras::status_summary(&state.db).await?))
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
    user: AuthUser,
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
            group: query.group_id,
            favorites_of: (query.favorites == Some(true)).then_some(user.id),
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
    if camera.tls_mode == Some(TlsMode::InsecureSkipVerify) {
        tracing::warn!(
            camera_id = %camera.summary.id,
            host = %camera.summary.host,
            admin = %admin.username,
            "camera configured with insecure-skip-verify: its TLS certificate will not be checked"
        );
    }
}

pub(super) fn settings_json(c: &CameraDetail) -> serde_json::Value {
    serde_json::json!({
        "name": c.summary.name,
        "host": c.summary.host,
        "port": c.summary.port,
        "scheme": c.summary.scheme,
        "tls_mode": c.tls_mode,
        "monitoring_enabled": c.summary.monitoring_enabled,
        "has_credentials": c.has_credentials,
        "credential_set": c.credential_set.as_ref().map(|s| &s.name),
        "allow_basic_over_http": c.allow_basic_over_http,
        "groups": c.summary.groups.iter().map(|g| &g.name).collect::<Vec<_>>(),
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
    JsonBody(mut body): JsonBody<NewCamera>,
) -> Result<(StatusCode, Json<CameraDetail>), ApiError> {
    let group_ids = std::mem::take(&mut body.group_ids);
    let (settings, credentials) =
        CameraSettings::from_new(body, &state.settings.current().camera_defaults)
            .map_err(|e| camera_error(e.into()))?;
    groups::ensure_exist(&state.db, &group_ids)
        .await
        .map_err(super::organization::org_error)?;
    let created = cameras::create(&state.db, &*state.credentials, &settings, credentials)
        .await
        .map_err(camera_error)?;
    let id = created.summary.id;
    if !group_ids.is_empty() {
        groups::set_for_camera(&state.db, id, &group_ids)
            .await
            .map_err(super::organization::org_error)?;
    }
    let camera = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
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
    JsonBody(mut body): JsonBody<CameraPatch>,
) -> Result<Json<CameraDetail>, ApiError> {
    let group_ids = body.group_ids.take();
    if let Some(g) = &group_ids {
        groups::ensure_exist(&state.db, g)
            .await
            .map_err(super::organization::org_error)?;
    }
    let defaults = state.settings.current().camera_defaults;
    let (before, _) = cameras::update(&state.db, &*state.credentials, id, body, &defaults)
        .await
        .map_err(camera_error)?;
    if let Some(g) = &group_ids {
        groups::set_for_camera(&state.db, id, g)
            .await
            .map_err(super::organization::org_error)?;
    }
    let after = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
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

/// Choose which stream profiles this camera uses for grid tiles and the
/// detail view.
///
/// Required role: admin. Each value is a profile id from the camera's
/// `stream_profiles` that browsers can play (H.264), or `null` to use the
/// global default (runtime settings, `streaming`), else a pick by resolution.
/// Streams already running keep their profile; new viewers get the new one.
#[utoipa::path(
    put,
    path = "/cameras/{id}/stream-profiles",
    operation_id = "set_camera_stream_profiles",
    tag = "cameras",
    params(("id" = Uuid, Path, description = "Camera ID")),
    request_body = StreamProfileOverrides,
    responses(
        (status = 200, description = "Updated camera", body = CameraDetail),
        (status = 400, description = "Not one of the camera's playable profiles", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
        (status = 404, description = "No such camera", body = Problem),
    )
)]
pub async fn set_stream_profiles(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<StreamProfileOverrides>,
) -> Result<Json<CameraDetail>, ApiError> {
    let before = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    for (field, choice) in [
        ("grid_profile", &body.grid_profile),
        ("detail_profile", &body.detail_profile),
    ] {
        if let Some(want) = choice {
            let ok = before
                .stream_profiles
                .iter()
                .any(|p| &p.id.0 == want && crate::streaming::profiles::playable(p));
            if !ok {
                return Err(ApiError::BadRequest(format!(
                    "{field} must be one of this camera's H.264 stream profiles (refresh the camera if its profiles changed)"
                )));
            }
        }
    }
    if !cameras::set_stream_overrides(&state.db, id, &body).await? {
        return Err(ApiError::NotFound);
    }
    let after = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::CAMERA_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", id)
            .ip(ip)
            .details(serde_json::json!({
                "from": { "grid_profile": before.grid_profile, "detail_profile": before.detail_profile },
                "to": { "grid_profile": after.grid_profile, "detail_profile": after.detail_profile },
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
    remove_camera(&state, &admin, ip, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a camera, stop its live streams, audit and announce it. Shared by
/// single and bulk delete.
pub(super) async fn remove_camera(
    state: &AppState,
    admin: &AdminUser,
    ip: Option<std::net::IpAddr>,
    id: Uuid,
) -> Result<(), ApiError> {
    let camera = cameras::get(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    cameras::delete(&state.db, id).await.map_err(camera_error)?;
    state.streams.stop_camera(id, "the camera was removed");
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
    Ok(())
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
    let target = cameras::target(&state.db, id)
        .await?
        .ok_or(ApiError::NotFound)?
        .provider_target(&*state.credentials)
        .map_err(|e| ApiError::Internal(format!("cannot decrypt camera credentials: {e}")))?;
    let provider = state.providers().create(&target);
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

/// Most cameras one bulk request may touch.
pub const MAX_BULK: usize = 500;

/// Connection and security settings to change on many cameras at once (issue
/// #19). Only the fields present change; everything else stays as it is.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct BulkSettings {
    /// New credentials (none, a shared set, or inline: each camera gets its
    /// own private copy).
    #[serde(default)]
    pub credentials: Option<CredentialInput>,
    #[serde(default)]
    pub allow_basic_over_http: Option<bool>,
    /// Certificate check for the HTTPS cameras among them (`pinned` needs a
    /// per-camera fingerprint, so it isn't offered here).
    #[serde(default)]
    pub tls_mode: Option<TlsMode>,
    #[serde(default)]
    pub scheme: Option<Scheme>,
    #[serde(default)]
    pub port: Option<u16>,
}

impl BulkSettings {
    fn is_empty(&self) -> bool {
        self.credentials.is_none()
            && self.allow_basic_over_http.is_none()
            && self.tls_mode.is_none()
            && self.scheme.is_none()
            && self.port.is_none()
    }

    /// What changes, for the audit log: never secrets.
    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "credentials": self.credentials.as_ref().map(CredentialInput::describe),
            "allow_basic_over_http": self.allow_basic_over_http,
            "tls_mode": self.tls_mode,
            "scheme": self.scheme,
            "port": self.port,
        })
    }

    fn patch_for(&self, camera: &CameraDetail) -> CameraPatch {
        let scheme = self.scheme.unwrap_or(camera.summary.scheme);
        CameraPatch {
            credentials: self.credentials.as_ref().map(CredentialInput::duplicate),
            allow_basic_over_http: self.allow_basic_over_http,
            // Certificate settings only mean something for HTTPS cameras.
            tls_mode: self.tls_mode.filter(|_| scheme == Scheme::Https),
            scheme: self.scheme,
            port: self.port,
            ..Default::default()
        }
    }
}

/// What to do to every selected camera.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkAction {
    AddGroups {
        group_ids: Vec<Uuid>,
    },
    RemoveGroups {
        group_ids: Vec<Uuid>,
    },
    SetMonitoring {
        enabled: bool,
    },
    /// Change connection and security settings (issue #19).
    UpdateSettings {
        settings: BulkSettings,
    },
    /// Remove the cameras (stops their streams and polling).
    Delete,
}

impl BulkAction {
    fn name(&self) -> &'static str {
        match self {
            BulkAction::AddGroups { .. } => "add_groups",
            BulkAction::RemoveGroups { .. } => "remove_groups",
            BulkAction::SetMonitoring { .. } => "set_monitoring",
            BulkAction::UpdateSettings { .. } => "update_settings",
            BulkAction::Delete => "delete",
        }
    }

    /// For the audit log; never contains secrets.
    fn describe(&self) -> serde_json::Value {
        match self {
            BulkAction::AddGroups { group_ids } | BulkAction::RemoveGroups { group_ids } => {
                serde_json::json!({ "type": self.name(), "group_ids": group_ids })
            }
            BulkAction::SetMonitoring { enabled } => {
                serde_json::json!({ "type": self.name(), "enabled": enabled })
            }
            BulkAction::UpdateSettings { settings } => {
                serde_json::json!({ "type": self.name(), "settings": settings.describe() })
            }
            BulkAction::Delete => serde_json::json!({ "type": self.name() }),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkRequest {
    /// Cameras by id. Together with `group_id` (both optional,
    /// combined as a union) the selection must come to 1–500 cameras.
    #[serde(default)]
    pub camera_ids: Vec<Uuid>,
    /// Every camera in this group.
    #[serde(default)]
    pub group_id: Option<Uuid>,
    /// With `group_id`: include cameras in its subgroups (default true).
    #[serde(default = "default_true")]
    pub include_subgroups: bool,
    pub action: BulkAction,
}

fn default_true() -> bool {
    true
}

/// A camera a bulk change skipped, and why.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkSkipped {
    pub camera_id: Uuid,
    pub reason: String,
}

/// `result` of a `bulk_cameras` task.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkResult {
    pub action: String,
    /// Cameras the action was applied to.
    pub processed: u32,
    /// The cameras the selection resolved to.
    pub camera_ids: Vec<Uuid>,
    /// Cameras left unchanged (settings changes only), with the reason.
    pub skipped: Vec<BulkSkipped>,
}

/// Resolve a bulk selection (ids, group) to unique camera ids.
async fn resolve_selection(state: &AppState, body: &BulkRequest) -> Result<Vec<Uuid>, ApiError> {
    use crate::organization::{ensure_cameras_exist, unique};
    let org = super::organization::org_error;
    let mut ids = body.camera_ids.clone();
    ensure_cameras_exist(&state.db, &unique(&ids))
        .await
        .map_err(org)?;
    if let Some(group) = body.group_id {
        groups::ensure_exist(&state.db, &[group])
            .await
            .map_err(org)?;
        let groups_in = if body.include_subgroups {
            groups::subtree(&state.db, group).await?
        } else {
            vec![group]
        };
        for g in groups_in {
            ids.extend(groups::direct_members(&state.db, g).await?);
        }
    }
    Ok(unique(&ids))
}

/// Apply one action to many cameras (group, monitoring, settings, delete).
///
/// Required role: admin. Select cameras by id, by group (optionally with its
/// subgroups). Validated up front (400 for unknown cameras or groups, or an
/// empty selection), then runs as a background task
/// (`bulk_cameras`) whose result is a `BulkResult`. Follow it via the `tasks`
/// topic. Cancelling stops between cameras; work already done stays done.
/// Settings changes are audited per camera (`camera.update`, never secrets)
/// and once for the whole request (`camera.bulk_update`).
#[utoipa::path(
    post,
    path = "/cameras/bulk",
    operation_id = "bulk_update_cameras",
    tag = "cameras",
    request_body = BulkRequest,
    responses(
        (status = 202, description = "Task started", body = crate::tasks::Task),
        (status = 400, description = "Invalid selection or action", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn bulk(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(body): JsonBody<BulkRequest>,
) -> Result<(StatusCode, Json<crate::tasks::Task>), ApiError> {
    let ids = resolve_selection(&state, &body).await?;
    if ids.is_empty() || ids.len() > MAX_BULK {
        return Err(ApiError::BadRequest(format!(
            "the selection must come to between 1 and {MAX_BULK} cameras (it has {})",
            ids.len()
        )));
    }
    let org = super::organization::org_error;
    match &body.action {
        BulkAction::AddGroups { group_ids } | BulkAction::RemoveGroups { group_ids } => {
            if group_ids.is_empty() {
                return Err(ApiError::BadRequest("choose at least one group".into()));
            }
            groups::ensure_exist(&state.db, group_ids)
                .await
                .map_err(org)?;
        }
        BulkAction::UpdateSettings { settings } => {
            if settings.is_empty() {
                return Err(ApiError::BadRequest(
                    "choose at least one setting to change".into(),
                ));
            }
            if settings.tls_mode == Some(TlsMode::Pinned) {
                return Err(ApiError::BadRequest(
                    "a pinned certificate is per camera; pin it on the camera itself".into(),
                ));
            }
            if let Some(CredentialInput::Set { id }) = &settings.credentials {
                if !crate::credentials::is_shared(&state.db, *id).await? {
                    return Err(ApiError::BadRequest(
                        "The selected credential set does not exist.".into(),
                    ));
                }
            }
        }
        BulkAction::SetMonitoring { .. } | BulkAction::Delete => {}
    }

    let creator = crate::tasks::Creator {
        id: admin.id,
        username: admin.username.clone(),
    };
    let action = body.action;
    let task_state = state.clone();
    let task = state
        .tasks
        .spawn("bulk_cameras", None, Some(creator), move |ctx| async move {
            let state = task_state;
            let total = u32::try_from(ids.len()).unwrap_or(u32::MAX);
            let fail = |e: ApiError| crate::tasks::TaskFailure::internal(e.to_string());
            let mut processed: u32 = 0;
            let mut skipped: Vec<BulkSkipped> = Vec::new();
            match &action {
                BulkAction::AddGroups { group_ids } => {
                    groups::add_cameras(&state.db, &ids, group_ids)
                        .await
                        .map_err(|e| fail(e.into()))?;
                }
                BulkAction::RemoveGroups { group_ids } => {
                    groups::remove_cameras(&state.db, &ids, group_ids)
                        .await
                        .map_err(|e| fail(e.into()))?;
                }
                BulkAction::SetMonitoring { .. }
                | BulkAction::UpdateSettings { .. }
                | BulkAction::Delete => {}
            }
            for id in &ids {
                if ctx.cancel.is_cancelled() {
                    break;
                }
                match &action {
                    BulkAction::Delete => match remove_camera(&state, &admin, ip, *id).await {
                        // Someone else removed it meanwhile: nothing left to do.
                        Ok(()) | Err(ApiError::NotFound) => {}
                        Err(e) => return Err(fail(e)),
                    },
                    BulkAction::SetMonitoring { enabled } => {
                        let patch = CameraPatch {
                            monitoring_enabled: Some(*enabled),
                            ..Default::default()
                        };
                        let defaults = state.settings.current().camera_defaults;
                        match cameras::update(&state.db, &*state.credentials, *id, patch, &defaults)
                            .await
                        {
                            Ok(_) | Err(CameraError::NotFound) => {}
                            Err(e) => return Err(fail(camera_error(e))),
                        }
                        super::organization::announce_cameras(&state, &[*id])
                            .await
                            .map_err(fail)?;
                    }
                    BulkAction::UpdateSettings { settings } => {
                        match update_one(&state, &admin, ip, *id, settings).await {
                            Ok(()) => {}
                            Err(reason) => {
                                skipped.push(BulkSkipped {
                                    camera_id: *id,
                                    reason,
                                });
                                continue;
                            }
                        }
                    }
                    _ => super::organization::announce_cameras(&state, &[*id])
                        .await
                        .map_err(fail)?,
                }
                processed += 1;
                if processed.is_multiple_of(10) || processed == total {
                    ctx.progress(processed, Some(total), "Updating cameras")
                        .await;
                }
            }
            if !matches!(action, BulkAction::Delete) {
                let details = serde_json::json!({
                    "action": action.describe(), "cameras": ids, "processed": processed,
                    "skipped": skipped.len(),
                });
                audit::record(
                    &state.db,
                    AuditEvent::new(action::CAMERA_BULK_UPDATE, Outcome::Success)
                        .actor(Some(admin.id), &admin.username)
                        .ip(ip)
                        .details(details),
                )
                .await
                .map_err(|e| crate::tasks::TaskFailure::internal(e.to_string()))?;
            }
            serde_json::to_value(BulkResult {
                action: action.name().to_owned(),
                processed,
                camera_ids: ids,
                skipped,
            })
            .map_err(|e| crate::tasks::TaskFailure::internal(e.to_string()))
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(task)))
}

/// Apply bulk settings to one camera. `Err` is a user-facing reason to skip it.
async fn update_one(
    state: &AppState,
    admin: &AdminUser,
    ip: Option<std::net::IpAddr>,
    id: Uuid,
    settings: &BulkSettings,
) -> Result<(), String> {
    let internal = |e: &dyn std::fmt::Display| format!("internal error: {e}");
    let Some(current) = cameras::get(&state.db, id)
        .await
        .map_err(|e| internal(&e))?
    else {
        return Err("the camera no longer exists".into());
    };
    let defaults = state.settings.current().camera_defaults;
    let patch = settings.patch_for(&current);
    let (before, after) =
        match cameras::update(&state.db, &*state.credentials, id, patch, &defaults).await {
            Ok(v) => v,
            Err(CameraError::Invalid(e)) => return Err(e.to_string()),
            Err(CameraError::UnknownCredentialSet) => {
                return Err("the selected credential set does not exist".into())
            }
            Err(CameraError::Duplicate) => {
                return Err("another camera already uses this address and port".into())
            }
            Err(CameraError::NotFound) => return Err("the camera no longer exists".into()),
            Err(e) => return Err(internal(&e)),
        };
    warn_if_insecure(&after, admin);
    audit::record(
        &state.db,
        AuditEvent::new(action::CAMERA_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("camera", id)
            .ip(ip)
            .details(serde_json::json!({
                "from": settings_json(&before),
                "to": settings_json(&after),
                "source": "bulk",
            })),
    )
    .await
    .map_err(|e| internal(&e))?;
    state.events.publish(Event::CameraChanged {
        camera: after.summary.clone(),
    });
    Ok(())
}
