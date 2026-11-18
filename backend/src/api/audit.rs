//! `/api/v1/audit`: read the audit log (admin only).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use utoipa::{IntoParams, ToSchema};

use super::error::{ApiError, Problem};
use super::extract::QueryParams;
use super::pagination::{Page, PageParams};
use crate::audit::{self, AuditEntry, Outcome};
use crate::auth::AdminUser;
use crate::AppState;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditQuery {
    /// 1-based page number (default 1).
    #[param(minimum = 1)]
    pub page: Option<u32>,
    /// Items per page (default 50, max 200).
    #[param(minimum = 1, maximum = 200)]
    pub page_size: Option<u32>,
    /// Exact action (e.g. `auth.login`) or a prefix ending in `.` (e.g. `user.`).
    pub action: Option<String>,
    /// Actor username (case-insensitive exact match).
    pub actor: Option<String>,
    #[param(inline)]
    pub outcome: Option<Outcome>,
    /// Only entries at or after this time (RFC 3339).
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[param(value_type = Option<String>, format = DateTime)]
    pub since: Option<OffsetDateTime>,
    /// Only entries before this time (RFC 3339).
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[param(value_type = Option<String>, format = DateTime)]
    pub until: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditList {
    pub items: Vec<AuditEntry>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// List audit log entries, newest first.
///
/// Required role: admin.
#[utoipa::path(
    get,
    path = "/audit",
    operation_id = "list_audit_entries",
    tag = "audit",
    params(AuditQuery),
    responses(
        (status = 200, description = "A page of audit entries", body = AuditList),
        (status = 400, description = "Invalid query", body = Problem),
        (status = 401, description = "Not signed in", body = Problem),
        (status = 403, description = "Not an admin", body = Problem),
    )
)]
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    QueryParams(query): QueryParams<AuditQuery>,
) -> Result<Json<AuditList>, ApiError> {
    let page = Page::try_from(PageParams {
        page: query.page,
        page_size: query.page_size,
    })?;
    let blank = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let action = blank(&query.action);
    let actor = blank(&query.actor);
    let (items, total) = audit::list(
        &state.db,
        audit::Filter {
            action: action.as_deref(),
            actor: actor.as_deref(),
            outcome: query.outcome,
            // Stored timestamps are UTC; compare like with like.
            since: query.since.map(|t| t.to_offset(UtcOffset::UTC)),
            until: query.until.map(|t| t.to_offset(UtcOffset::UTC)),
        },
        page.limit(),
        page.offset(),
    )
    .await?;
    Ok(Json(AuditList {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    }))
}
