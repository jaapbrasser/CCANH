//! `GET /api/v1/health`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::Mode;
use crate::AppState;

#[derive(Debug, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    /// Short, non-sensitive description when not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthChecks {
    pub database: ComponentHealth,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
    /// Server version (crate version).
    pub version: String,
    pub mode: Mode,
    pub uptime_seconds: u64,
    pub checks: HealthChecks,
}

/// Service health.
///
/// Required role: none (public). Returns 200 when every component is healthy and
/// 503 otherwise. Background services will report here as they are added.
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "All components healthy", body = HealthResponse),
        (status = 503, description = "One or more components unhealthy", body = HealthResponse),
    )
)]
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database = match crate::storage::ping(&state.db).await {
        Ok(()) => ComponentHealth {
            status: HealthStatus::Ok,
            detail: None,
        },
        Err(err) => {
            tracing::warn!(error = %err, "database health check failed");
            ComponentHealth {
                status: HealthStatus::Degraded,
                detail: Some("database unreachable".into()),
            }
        }
    };

    let status = database.status;
    let code = match status {
        HealthStatus::Ok => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::SERVICE_UNAVAILABLE,
    };

    (
        code,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            mode: state.config.mode,
            uptime_seconds: state.started_at.elapsed().as_secs(),
            checks: HealthChecks { database },
        }),
    )
}
