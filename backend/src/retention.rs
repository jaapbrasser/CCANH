//! History retention (SPEC §11.3): a supervised background job that deletes
//! status history, finished tasks (with their discovery scans) and, if an
//! admin opted in, old audit entries. Runs shortly after startup, then daily.

use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;

use crate::audit::{self, action, Event as AuditEvent, Outcome};
use crate::settings::{RetentionSettings, SettingsHandle};

/// Delay before the first cleanup after startup, so it doesn't compete with
/// the startup burst of status checks.
pub const FIRST_RUN_DELAY: Duration = Duration::from_secs(60);
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Rows removed by one cleanup.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RetentionReport {
    pub status_history: u64,
    pub tasks: u64,
    pub audit: u64,
}

fn days_ago(now: OffsetDateTime, days: u32) -> OffsetDateTime {
    now - time::Duration::days(i64::from(days))
}

/// Apply `settings` once, as of `now`.
pub async fn run_once(
    db: &SqlitePool,
    settings: &RetentionSettings,
    now: OffsetDateTime,
) -> Result<RetentionReport, sqlx::Error> {
    let history_cutoff = days_ago(now, settings.status_history_days);
    let status_history = sqlx::query!(
        "DELETE FROM camera_status_history WHERE occurred_at < ?1",
        history_cutoff
    )
    .execute(db)
    .await?
    .rows_affected();

    // Discovery jobs and their results cascade with their task.
    let tasks_cutoff = days_ago(now, settings.tasks_days);
    let tasks = sqlx::query!(
        "DELETE FROM tasks WHERE status IN ('succeeded', 'failed', 'cancelled') AND completed_at < ?1",
        tasks_cutoff
    )
    .execute(db)
    .await?
    .rows_affected();

    let audit = if settings.audit_days == 0 {
        0
    } else {
        let cutoff = days_ago(now, settings.audit_days);
        let n = sqlx::query!("DELETE FROM audit_log WHERE occurred_at < ?1", cutoff)
            .execute(db)
            .await?
            .rows_affected();
        if n > 0 {
            // Removing audit entries is itself recorded.
            audit::record(
                db,
                AuditEvent::new(action::AUDIT_PURGE, Outcome::Success).details(serde_json::json!({
                    "removed": n, "older_than_days": settings.audit_days,
                })),
            )
            .await?;
        }
        n
    };

    // Let SQLite refresh its query planner statistics after big deletes.
    sqlx::query("PRAGMA optimize").execute(db).await?;
    Ok(RetentionReport {
        status_history,
        tasks,
        audit,
    })
}

/// The retention service: first run after `first_delay`, then every `interval`.
pub async fn run(
    db: SqlitePool,
    settings: SettingsHandle,
    first_delay: Duration,
    interval: Duration,
    cancel: CancellationToken,
) {
    let mut wait = first_delay;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
        wait = interval;
        let current = settings.current().retention;
        match run_once(&db, &current, OffsetDateTime::now_utc()).await {
            Ok(r) if r == RetentionReport::default() => {
                tracing::debug!("retention: nothing to remove")
            }
            Ok(r) => tracing::info!(
                status_history = r.status_history,
                tasks = r.tasks,
                audit = r.audit,
                "retention: removed old history"
            ),
            Err(e) => {
                tracing::warn!(error = %e, "retention cleanup failed; retrying at the next run")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    async fn seed(db: &SqlitePool, now: OffsetDateTime) -> Uuid {
        // A camera with history 10 and 40 days old.
        let cam = Uuid::now_v7();
        let hc = cam.hyphenated();
        sqlx::query(
            "INSERT INTO cameras (id, name, host, port, scheme, tls_mode, monitoring_enabled, created_at, updated_at)
             VALUES (?1, 'Cam', '192.0.2.1', 443, 'https', 'strict', 0, ?2, ?2)",
        )
        .bind(hc.to_string())
        .bind(now)
        .execute(db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO camera_status (camera_id) VALUES (?1)")
            .bind(hc.to_string())
            .execute(db)
            .await
            .unwrap();
        for days in [10, 40] {
            sqlx::query(
                "INSERT INTO camera_status_history (camera_id, occurred_at, from_state, to_state)
                 VALUES (?1, ?2, 'unknown', 'online')",
            )
            .bind(hc.to_string())
            .bind(days_ago(now, days))
            .execute(db)
            .await
            .unwrap();
        }
        // Tasks finished 10 and 40 days ago, and one still running.
        for (days, status) in [(10, "succeeded"), (40, "failed"), (40, "running")] {
            let done = (status != "running").then(|| days_ago(now, days));
            sqlx::query(
                "INSERT INTO tasks (id, kind, status, progress_current, created_at, completed_at)
                 VALUES (?1, 'test', ?2, 0, ?3, ?4)",
            )
            .bind(Uuid::now_v7().hyphenated().to_string())
            .bind(status)
            .bind(days_ago(now, days))
            .bind(done)
            .execute(db)
            .await
            .unwrap();
        }
        // Audit entries 10 and 400 days old.
        for days in [10, 400] {
            sqlx::query(
                "INSERT INTO audit_log (occurred_at, action, outcome) VALUES (?1, 'auth.login', 'success')",
            )
            .bind(days_ago(now, days))
            .execute(db)
            .await
            .unwrap();
        }
        cam
    }

    async fn count(db: &SqlitePool, table: &str) -> i64 {
        let sql = match table {
            "camera_status_history" => "SELECT COUNT(*) FROM camera_status_history",
            "tasks" => "SELECT COUNT(*) FROM tasks",
            "audit_log" => "SELECT COUNT(*) FROM audit_log",
            other => panic!("unexpected table {other}"),
        };
        sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
    }

    #[tokio::test]
    async fn removes_only_what_is_older_than_configured() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let now = OffsetDateTime::now_utc();
        seed(&db, now).await;
        let keep_audit = RetentionSettings {
            status_history_days: 30,
            tasks_days: 30,
            audit_days: 0,
        };
        let r = run_once(&db, &keep_audit, now).await.unwrap();
        assert_eq!(
            r,
            RetentionReport {
                status_history: 1,
                tasks: 1,
                audit: 0
            }
        );
        assert_eq!(count(&db, "camera_status_history").await, 1);
        assert_eq!(
            count(&db, "tasks").await,
            2,
            "running tasks are never removed"
        );
        assert_eq!(
            count(&db, "audit_log").await,
            2,
            "audit kept forever by default"
        );

        let year = RetentionSettings {
            audit_days: 365,
            ..keep_audit
        };
        let r = run_once(&db, &year, now).await.unwrap();
        assert_eq!(r.audit, 1);
        let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(
            actions,
            ["auth.login", "audit.purge"],
            "the purge is audited"
        );
        assert_eq!(
            run_once(&db, &year, now).await.unwrap(),
            RetentionReport::default()
        );
    }

    #[tokio::test]
    async fn service_waits_for_the_first_delay() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let now = OffsetDateTime::now_utc();
        seed(&db, now).await;
        let handle = SettingsHandle::new(crate::settings::RuntimeSettings::from_config(
            &crate::config::Config::default(),
        ));
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(
            db.clone(),
            handle,
            Duration::from_millis(300),
            Duration::from_secs(3600),
            cancel.clone(),
        ));
        assert_eq!(count(&db, "camera_status_history").await, 2, "not yet");
        let mut removed = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if count(&db, "camera_status_history").await == 1 {
                removed = true;
                break;
            }
        }
        assert!(removed, "the first cleanup ran after its delay");
        cancel.cancel();
        task.await.unwrap();
    }
}
