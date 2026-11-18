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
use crate::settings::{MonitoringPatch, MonitoringSettings, RuntimeSettings, SettingsError};
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
