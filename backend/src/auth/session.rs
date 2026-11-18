//! Server-side sessions stored in SQLite.
//!
//! The browser holds a random 256-bit token in an `HttpOnly` cookie; the database
//! stores only its SHA-256. A session ends when it is idle for longer than the
//! idle timeout, when it reaches its absolute expiry, on logout, or when the
//! user's password changes or they are disabled or deleted.

use std::net::IpAddr;
use std::time::Duration;

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use super::{hash_token, new_token, RandomError, Role};
use crate::config::AuthConfig;

pub const COOKIE_NAME: &str = "crane_session";

/// Only write `last_seen_at` when it is at least this stale, so ordinary browsing
/// doesn't cause a database write per request.
const TOUCH_INTERVAL_SECS: i64 = 60;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub idle: Duration,
    pub absolute: Duration,
}

impl From<&AuthConfig> for Timeouts {
    fn from(c: &AuthConfig) -> Self {
        Self {
            idle: Duration::from_secs(c.idle_timeout_minutes * 60),
            absolute: Duration::from_secs(c.absolute_timeout_hours * 60 * 60),
        }
    }
}

fn secs(d: Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Random(#[from] RandomError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// The user behind a valid session.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user_id: Uuid,
    pub username: String,
    pub role: Role,
    pub token_hash: String,
}

/// Create a session and return the raw token for the cookie.
pub async fn create(
    db: &SqlitePool,
    user_id: Uuid,
    source_ip: Option<IpAddr>,
    user_agent: Option<&str>,
    timeouts: Timeouts,
) -> Result<String, SessionError> {
    let token = new_token()?;
    let token_hash = hash_token(&token);
    let user_id = user_id.hyphenated();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let expires_at = now.saturating_add(secs(timeouts.absolute));
    let ip = source_ip.map(|ip| ip.to_string());
    let user_agent = user_agent.map(|ua| ua.chars().take(256).collect::<String>());
    sqlx::query!(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_seen_at, expires_at, source_ip, user_agent)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6)",
        token_hash,
        user_id,
        now,
        expires_at,
        ip,
        user_agent
    )
    .execute(db)
    .await?;
    Ok(token)
}

/// Resolve a raw token to its user, enforcing both timeouts and the user's
/// disabled flag. Expired sessions are deleted on sight.
pub async fn validate(
    db: &SqlitePool,
    token: &str,
    timeouts: Timeouts,
) -> Result<Option<SessionUser>, sqlx::Error> {
    validate_at(db, token, timeouts, OffsetDateTime::now_utc()).await
}

pub async fn validate_at(
    db: &SqlitePool,
    token: &str,
    timeouts: Timeouts,
    now: OffsetDateTime,
) -> Result<Option<SessionUser>, sqlx::Error> {
    let token_hash = hash_token(token);
    let now = now.unix_timestamp();
    let row = sqlx::query!(
        r#"SELECT s.last_seen_at, s.expires_at, u.id AS "user_id: Hyphenated", u.username,
                  u.role AS "role: Role", u.disabled AS "disabled: bool"
             FROM sessions s JOIN users u ON u.id = s.user_id
            WHERE s.token_hash = ?1"#,
        token_hash
    )
    .fetch_optional(db)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let idle_expired = now >= row.last_seen_at.saturating_add(secs(timeouts.idle));
    if idle_expired || now >= row.expires_at || row.disabled {
        delete(db, &token_hash).await?;
        return Ok(None);
    }

    if now - row.last_seen_at >= TOUCH_INTERVAL_SECS {
        sqlx::query!(
            "UPDATE sessions SET last_seen_at = ?2 WHERE token_hash = ?1",
            token_hash,
            now
        )
        .execute(db)
        .await?;
    }

    Ok(Some(SessionUser {
        user_id: row.user_id.into_uuid(),
        username: row.username,
        role: row.role,
        token_hash,
    }))
}

pub async fn delete(db: &SqlitePool, token_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM sessions WHERE token_hash = ?1", token_hash)
        .execute(db)
        .await?;
    Ok(())
}

/// End all of a user's sessions, optionally keeping one (the caller's own).
pub async fn delete_for_user(
    db: &SqlitePool,
    user_id: Uuid,
    keep_token_hash: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let user_id = user_id.hyphenated();
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE user_id = ?1 AND (?2 IS NULL OR token_hash != ?2)",
        user_id,
        keep_token_hash
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Delete expired sessions. Run periodically by the background sweeper.
pub async fn purge_expired(db: &SqlitePool, timeouts: Timeouts) -> Result<u64, sqlx::Error> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let idle_cutoff = now.saturating_sub(secs(timeouts.idle));
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE expires_at <= ?1 OR last_seen_at <= ?2",
        now,
        idle_cutoff
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{storage, users};

    const T: Timeouts = Timeouts {
        idle: Duration::from_secs(8 * 3600),
        absolute: Duration::from_secs(7 * 24 * 3600),
    };

    async fn setup() -> (SqlitePool, Uuid) {
        let db = storage::connect_in_memory().await.unwrap();
        let user = users::create(&db, "alice", "h", Role::Viewer)
            .await
            .unwrap();
        (db, user.id)
    }

    async fn count(db: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_and_validate() {
        let (db, user_id) = setup().await;
        let token = create(&db, user_id, None, Some("test"), T).await.unwrap();
        let session = validate(&db, &token, T).await.unwrap().unwrap();
        assert_eq!(session.user_id, user_id);
        assert_eq!(session.username, "alice");
        assert_eq!(session.role, Role::Viewer);

        // The raw token is never stored.
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM sessions")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_ne!(stored, token);
        assert!(validate(&db, "wrong-token", T).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn idle_timeout_expires_and_deletes() {
        let (db, user_id) = setup().await;
        let token = create(&db, user_id, None, None, T).await.unwrap();
        let later =
            OffsetDateTime::now_utc() + time::Duration::hours(8) + time::Duration::seconds(1);
        assert!(validate_at(&db, &token, T, later).await.unwrap().is_none());
        assert_eq!(count(&db).await, 0);
    }

    #[tokio::test]
    async fn activity_extends_idle_but_not_absolute() {
        let (db, user_id) = setup().await;
        let token = create(&db, user_id, None, None, T).await.unwrap();
        let start = OffsetDateTime::now_utc();
        // Keep the session active every 7 hours for 7 days.
        let mut t = start;
        while t < start + time::Duration::days(7) - time::Duration::hours(7) {
            t += time::Duration::hours(7);
            assert!(
                validate_at(&db, &token, T, t).await.unwrap().is_some(),
                "should still be valid at {t}"
            );
        }
        let past_absolute = start + time::Duration::days(7) + time::Duration::seconds(1);
        assert!(validate_at(&db, &token, T, past_absolute)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn disabled_user_session_is_rejected() {
        let (db, user_id) = setup().await;
        let token = create(&db, user_id, None, None, T).await.unwrap();
        users::update(&db, user_id, None, Some(true)).await.unwrap();
        assert!(validate(&db, &token, T).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_for_user_can_keep_current() {
        let (db, user_id) = setup().await;
        let keep = create(&db, user_id, None, None, T).await.unwrap();
        create(&db, user_id, None, None, T).await.unwrap();
        create(&db, user_id, None, None, T).await.unwrap();
        let removed = delete_for_user(&db, user_id, Some(&hash_token(&keep)))
            .await
            .unwrap();
        assert_eq!(removed, 2);
        assert!(validate(&db, &keep, T).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn sessions_cascade_on_user_delete() {
        let (db, user_id) = setup().await;
        create(&db, user_id, None, None, T).await.unwrap();
        users::delete(&db, user_id).await.unwrap();
        assert_eq!(count(&db).await, 0);
    }

    #[tokio::test]
    async fn purge_removes_only_expired() {
        let (db, user_id) = setup().await;
        let live = create(&db, user_id, None, None, T).await.unwrap();
        let stale = create(&db, user_id, None, None, T).await.unwrap();
        sqlx::query("UPDATE sessions SET last_seen_at = 0 WHERE token_hash = ?")
            .bind(hash_token(&stale))
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(purge_expired(&db, T).await.unwrap(), 1);
        assert!(validate(&db, &live, T).await.unwrap().is_some());
    }
}
