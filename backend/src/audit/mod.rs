//! Append-only audit log of security-relevant actions (SPEC §11.2).
//!
//! Entries record who did what to which target, from where, and whether it
//! succeeded. `details` is a small JSON object and must never contain secrets.

use std::net::IpAddr;

use serde::Serialize;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

/// Stable action names. Add new ones here so they stay discoverable.
pub mod action {
    pub const LOGIN: &str = "auth.login";
    pub const LOGOUT: &str = "auth.logout";
    pub const PASSWORD_CHANGE: &str = "auth.password_change";
    pub const SETUP_COMPLETE: &str = "setup.complete";
    pub const USER_CREATE: &str = "user.create";
    pub const USER_UPDATE: &str = "user.update";
    pub const USER_DELETE: &str = "user.delete";
    pub const USER_PASSWORD_RESET: &str = "user.password_reset";
    pub const CAMERA_CREATE: &str = "camera.create";
    pub const CAMERA_UPDATE: &str = "camera.update";
    pub const CAMERA_DELETE: &str = "camera.delete";
    pub const CREDENTIAL_SET_CREATE: &str = "credential_set.create";
    pub const CREDENTIAL_SET_UPDATE: &str = "credential_set.update";
    pub const CREDENTIAL_SET_DELETE: &str = "credential_set.delete";
    pub const SETTINGS_UPDATE: &str = "settings.update";
    pub const STREAM_FORCE_STOP: &str = "stream.force_stop";
    pub const CAMERA_BULK_UPDATE: &str = "camera.bulk_update";
    pub const DISCOVERY_START: &str = "discovery.start";
    pub const AUDIT_PURGE: &str = "audit.purge";
    pub const GROUP_CREATE: &str = "group.create";
    pub const GROUP_UPDATE: &str = "group.update";
    pub const GROUP_DELETE: &str = "group.delete";
    pub const TAG_CREATE: &str = "tag.create";
    pub const TAG_UPDATE: &str = "tag.update";
    pub const TAG_DELETE: &str = "tag.delete";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
        }
    }
}

/// An audit event under construction.
#[derive(Debug, Clone)]
pub struct Event {
    pub action: &'static str,
    pub outcome: Outcome,
    pub actor_id: Option<Uuid>,
    pub actor_username: Option<String>,
    pub target_type: Option<&'static str>,
    pub target_id: Option<String>,
    pub source_ip: Option<IpAddr>,
    pub details: Option<serde_json::Value>,
}

impl Event {
    pub fn new(action: &'static str, outcome: Outcome) -> Self {
        Self {
            action,
            outcome,
            actor_id: None,
            actor_username: None,
            target_type: None,
            target_id: None,
            source_ip: None,
            details: None,
        }
    }

    pub fn actor(mut self, id: Option<Uuid>, username: impl Into<String>) -> Self {
        self.actor_id = id;
        // Attempted usernames come from untrusted input; bound their size.
        self.actor_username = Some(username.into().chars().take(128).collect());
        self
    }

    pub fn target(mut self, kind: &'static str, id: impl ToString) -> Self {
        self.target_type = Some(kind);
        self.target_id = Some(id.to_string());
        self
    }

    pub fn ip(mut self, ip: Option<IpAddr>) -> Self {
        self.source_ip = ip;
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

pub async fn record(db: &SqlitePool, event: Event) -> Result<(), sqlx::Error> {
    let now = OffsetDateTime::now_utc();
    let actor_id = event.actor_id.map(|id| id.hyphenated().to_string());
    let outcome = event.outcome.as_str();
    let ip = event.source_ip.map(|ip| ip.to_string());
    let details = event.details.map(|d| d.to_string());
    sqlx::query!(
        "INSERT INTO audit_log (occurred_at, actor_id, actor_username, action, outcome, target_type, target_id, source_ip, details)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        now,
        actor_id,
        event.actor_username,
        event.action,
        outcome,
        event.target_type,
        event.target_id,
        ip,
        details
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Record an event where a failed write must not fail the request (e.g. a failed
/// login, which must still return 401). The failure is logged instead.
pub async fn record_best_effort(db: &SqlitePool, event: Event) {
    let action = event.action;
    if let Err(err) = record(db, event).await {
        tracing::error!(error = %err, action, "failed to write audit log entry");
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditEntry {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    pub actor_id: Option<String>,
    pub actor_username: Option<String>,
    pub action: String,
    pub outcome: Outcome,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub source_ip: Option<String>,
    /// Structured, non-secret context for the action.
    #[schema(value_type = Option<Object>)]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Default)]
pub struct Filter<'a> {
    /// Exact action, or a prefix ending in `.` (e.g. `user.`).
    pub action: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub outcome: Option<Outcome>,
    pub since: Option<OffsetDateTime>,
    pub until: Option<OffsetDateTime>,
}

/// Newest first.
pub async fn list(
    db: &SqlitePool,
    filter: Filter<'_>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AuditEntry>, i64), sqlx::Error> {
    let action_prefix = filter
        .action
        .filter(|a| a.ends_with('.'))
        .map(|a| format!("{a}%"));
    let action_exact = filter.action.filter(|a| !a.ends_with('.'));
    let outcome = filter.outcome.map(Outcome::as_str);

    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM audit_log
            WHERE (?1 IS NULL OR action = ?1)
              AND (?2 IS NULL OR action LIKE ?2)
              AND (?3 IS NULL OR actor_username = ?3 COLLATE NOCASE)
              AND (?4 IS NULL OR outcome = ?4)
              AND (?5 IS NULL OR occurred_at >= ?5)
              AND (?6 IS NULL OR occurred_at < ?6)"#,
        action_exact,
        action_prefix,
        filter.actor,
        outcome,
        filter.since,
        filter.until
    )
    .fetch_one(db)
    .await?;

    let rows = sqlx::query!(
        r#"SELECT id AS "id!: i64", occurred_at AS "occurred_at: OffsetDateTime", actor_id,
                  actor_username, action, outcome, target_type, target_id, source_ip, details
             FROM audit_log
            WHERE (?1 IS NULL OR action = ?1)
              AND (?2 IS NULL OR action LIKE ?2)
              AND (?3 IS NULL OR actor_username = ?3 COLLATE NOCASE)
              AND (?4 IS NULL OR outcome = ?4)
              AND (?5 IS NULL OR occurred_at >= ?5)
              AND (?6 IS NULL OR occurred_at < ?6)
            ORDER BY id DESC
            LIMIT ?7 OFFSET ?8"#,
        action_exact,
        action_prefix,
        filter.actor,
        outcome,
        filter.since,
        filter.until,
        limit,
        offset
    )
    .fetch_all(db)
    .await?;

    let entries = rows
        .into_iter()
        .map(|r| AuditEntry {
            id: r.id,
            occurred_at: r.occurred_at,
            actor_id: r.actor_id,
            actor_username: r.actor_username,
            action: r.action,
            outcome: if r.outcome == "success" {
                Outcome::Success
            } else {
                Outcome::Failure
            },
            target_type: r.target_type,
            target_id: r.target_id,
            source_ip: r.source_ip,
            details: r.details.and_then(|d| serde_json::from_str(&d).ok()),
        })
        .collect();
    Ok((entries, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    #[tokio::test]
    async fn record_and_filter() {
        let db = storage::connect_in_memory().await.unwrap();
        record(
            &db,
            Event::new(action::LOGIN, Outcome::Failure)
                .actor(None, "mallory")
                .ip(Some(IpAddr::from([10, 0, 0, 5])))
                .details(serde_json::json!({ "reason": "unknown_user" })),
        )
        .await
        .unwrap();
        record(
            &db,
            Event::new(action::USER_CREATE, Outcome::Success)
                .actor(Some(Uuid::now_v7()), "admin")
                .target("user", "abc"),
        )
        .await
        .unwrap();

        let (all, total) = list(&db, Filter::default(), 50, 0).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(all[0].action, action::USER_CREATE, "newest first");

        let (failures, _) = list(
            &db,
            Filter {
                outcome: Some(Outcome::Failure),
                ..Default::default()
            },
            50,
            0,
        )
        .await
        .unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].source_ip.as_deref(), Some("10.0.0.5"));
        assert_eq!(
            failures[0].details.as_ref().unwrap()["reason"],
            "unknown_user"
        );

        let (by_prefix, _) = list(
            &db,
            Filter {
                action: Some("user."),
                ..Default::default()
            },
            50,
            0,
        )
        .await
        .unwrap();
        assert_eq!(by_prefix.len(), 1);

        let (by_actor, _) = list(
            &db,
            Filter {
                actor: Some("ADMIN"),
                ..Default::default()
            },
            50,
            0,
        )
        .await
        .unwrap();
        assert_eq!(by_actor.len(), 1);
    }

    #[tokio::test]
    async fn audit_log_is_append_only() {
        let db = storage::connect_in_memory().await.unwrap();
        record(&db, Event::new(action::LOGOUT, Outcome::Success))
            .await
            .unwrap();
        let err = sqlx::query("UPDATE audit_log SET action = 'tampered'")
            .execute(&db)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }
}
