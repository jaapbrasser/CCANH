//! `/api/v1/settings/*`: admin-editable runtime settings (SPEC §3.4).

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::error::{ApiError, Problem};
use super::extract::JsonBody;
use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::auth::{AdminUser, ClientIp};
use crate::monitoring::MonitorStatsSnapshot;
use crate::settings::{
    CameraDefaults, CameraDefaultsPatch, DiscoveryPatch, DiscoverySettings, MonitoringPatch,
    MonitoringSettings, RetentionPatch, RetentionSettings, RuntimeSettings, SettingsError,
    StreamingPatch, StreamingSettings,
};
use crate::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsView {
    /// Values in effect now.
    pub current: RuntimeSettings,
    /// Startup defaults (from the config file / environment).
    pub defaults: RuntimeSettings,
    /// Live counters of the status monitor.
    pub monitor: MonitorStatsSnapshot,
}

fn settings_error(e: SettingsError) -> ApiError {
    match e {
        SettingsError::Invalid(msg) => ApiError::BadRequest(msg),
        SettingsError::Database(e) => ApiError::from(e),
    }
}

/// Current runtime settings, defaults and monitor counters.
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/settings",
    operation_id = "get_settings",
    tag = "settings",
    responses(
        (status = 200, description = "Settings", body = SettingsView),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn get(_admin: AdminUser, State(state): State<AppState>) -> Json<SettingsView> {
    Json(SettingsView {
        current: state.settings.current(),
        defaults: state.settings.defaults().clone(),
        monitor: state.monitor_stats.snapshot(),
    })
}

/// Change camera monitoring settings.
///
/// Required role: admin. Takes effect within a second, without a restart.
/// Audited with the old and new values.
#[utoipa::path(
    patch,
    path = "/settings/monitoring",
    operation_id = "update_monitoring_settings",
    tag = "settings",
    request_body = MonitoringPatch,
    responses(
        (status = 200, description = "New monitoring settings", body = MonitoringSettings),
        (status = 400, description = "A value is out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn update_monitoring(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(patch): JsonBody<MonitoringPatch>,
) -> Result<Json<MonitoringSettings>, ApiError> {
    let (before, after) = state
        .settings
        .update_monitoring(&state.db, &patch, &admin.username)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "monitoring")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after })),
    )
    .await?;
    Ok(Json(after))
}

/// Reset camera monitoring settings to the startup defaults.
///
/// Required role: admin.
#[utoipa::path(
    delete,
    path = "/settings/monitoring",
    operation_id = "reset_monitoring_settings",
    tag = "settings",
    responses(
        (status = 200, description = "Default monitoring settings, now in effect", body = MonitoringSettings),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn reset_monitoring(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<MonitoringSettings>, ApiError> {
    let before = state.settings.current().monitoring;
    let after = state
        .settings
        .reset_monitoring(&state.db)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "monitoring")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after, "reset": true })),
    )
    .await?;
    Ok(Json(after))
}

/// Change live video settings (grace period, buffer caps).
///
/// Required role: admin. Applies to streams immediately. Audited.
#[utoipa::path(
    patch,
    path = "/settings/streaming",
    operation_id = "update_streaming_settings",
    tag = "settings",
    request_body = StreamingPatch,
    responses(
        (status = 200, description = "New streaming settings", body = StreamingSettings),
        (status = 400, description = "A value is out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn update_streaming(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(patch): JsonBody<StreamingPatch>,
) -> Result<Json<StreamingSettings>, ApiError> {
    let (before, after) = state
        .settings
        .update_streaming(&state.db, &patch, &admin.username)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "streaming")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after })),
    )
    .await?;
    Ok(Json(after))
}

/// Reset live video settings to the startup defaults.
///
/// Required role: admin.
#[utoipa::path(
    delete,
    path = "/settings/streaming",
    operation_id = "reset_streaming_settings",
    tag = "settings",
    responses(
        (status = 200, description = "Default streaming settings, now in effect", body = StreamingSettings),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn reset_streaming(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<StreamingSettings>, ApiError> {
    let before = state.settings.current().streaming;
    let after = state
        .settings
        .reset_streaming(&state.db)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "streaming")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after, "reset": true })),
    )
    .await?;
    Ok(Json(after))
}

/// Change discovery settings (address cap, concurrency, timeouts).
///
/// Required role: admin. Applies to the next scan. Audited.
#[utoipa::path(
    patch,
    path = "/settings/discovery",
    operation_id = "update_discovery_settings",
    tag = "settings",
    request_body = DiscoveryPatch,
    responses(
        (status = 200, description = "New discovery settings", body = DiscoverySettings),
        (status = 400, description = "A value is out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn update_discovery(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(patch): JsonBody<DiscoveryPatch>,
) -> Result<Json<DiscoverySettings>, ApiError> {
    let (before, after) = state
        .settings
        .update_discovery(&state.db, &patch, &admin.username)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "discovery")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after })),
    )
    .await?;
    Ok(Json(after))
}

/// Reset discovery settings to the startup defaults.
///
/// Required role: admin.
#[utoipa::path(
    delete,
    path = "/settings/discovery",
    operation_id = "reset_discovery_settings",
    tag = "settings",
    responses(
        (status = 200, description = "Default discovery settings, now in effect", body = DiscoverySettings),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn reset_discovery(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<DiscoverySettings>, ApiError> {
    let before = state.settings.current().discovery;
    let after = state
        .settings
        .reset_discovery(&state.db)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "discovery")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after, "reset": true })),
    )
    .await?;
    Ok(Json(after))
}

/// Change how long history is kept (status history, tasks and scans, audit log).
///
/// Required role: admin. Applies at the next daily cleanup. Audited.
#[utoipa::path(
    patch,
    path = "/settings/retention",
    operation_id = "update_retention_settings",
    tag = "settings",
    request_body = RetentionPatch,
    responses(
        (status = 200, description = "New retention settings", body = RetentionSettings),
        (status = 400, description = "A value is out of range", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn update_retention(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(patch): JsonBody<RetentionPatch>,
) -> Result<Json<RetentionSettings>, ApiError> {
    let (before, after) = state
        .settings
        .update_retention(&state.db, &patch, &admin.username)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "retention")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after })),
    )
    .await?;
    Ok(Json(after))
}

/// Reset retention settings to the startup defaults.
///
/// Required role: admin.
#[utoipa::path(
    delete,
    path = "/settings/retention",
    operation_id = "reset_retention_settings",
    tag = "settings",
    responses(
        (status = 200, description = "Default retention settings, now in effect", body = RetentionSettings),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn reset_retention(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<RetentionSettings>, ApiError> {
    let before = state.settings.current().retention;
    let after = state
        .settings
        .reset_retention(&state.db)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "retention")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after, "reset": true })),
    )
    .await?;
    Ok(Json(after))
}

/// Change the defaults new cameras get (scheme, Basic over HTTP, certificate check).
///
/// Required role: admin. Only affects cameras added afterwards; existing
/// cameras keep their settings. Audited.
#[utoipa::path(
    patch,
    path = "/settings/camera-defaults",
    operation_id = "update_camera_defaults",
    tag = "settings",
    request_body = CameraDefaultsPatch,
    responses(
        (status = 200, description = "New camera defaults", body = CameraDefaults),
        (status = 400, description = "Invalid value", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn update_camera_defaults(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    JsonBody(patch): JsonBody<CameraDefaultsPatch>,
) -> Result<Json<CameraDefaults>, ApiError> {
    let (before, after) = state
        .settings
        .update_camera_defaults(&state.db, &patch, &admin.username)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "camera_defaults")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after })),
    )
    .await?;
    Ok(Json(after))
}

/// Reset the camera defaults to the startup defaults.
///
/// Required role: admin. Audited.
#[utoipa::path(
    delete,
    path = "/settings/camera-defaults",
    operation_id = "reset_camera_defaults",
    tag = "settings",
    responses(
        (status = 200, description = "Default camera defaults, now in effect", body = CameraDefaults),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn reset_camera_defaults(
    admin: AdminUser,
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<Json<CameraDefaults>, ApiError> {
    let before = state.settings.current().camera_defaults;
    let after = state
        .settings
        .reset_camera_defaults(&state.db)
        .await
        .map_err(settings_error)?;
    audit::record(
        &state.db,
        AuditEvent::new(action::SETTINGS_UPDATE, Outcome::Success)
            .actor(Some(admin.id), &admin.username)
            .target("settings", "camera_defaults")
            .ip(ip)
            .details(serde_json::json!({ "from": before, "to": after, "reset": true })),
    )
    .await?;
    Ok(Json(after))
}
