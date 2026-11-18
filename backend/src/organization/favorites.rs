//! Per-user favorite cameras. Never visible to or affecting other users.

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use super::OrgError;

/// The user's favorite camera ids, most recently added first.
pub async fn list(db: &SqlitePool, user: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let huser = user.hyphenated();
    let ids = sqlx::query_scalar!(
        r#"SELECT camera_id AS "id: Hyphenated" FROM favorites
            WHERE user_id = ?1 ORDER BY created_at DESC, camera_id"#,
        huser
    )
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().map(Hyphenated::into_uuid).collect())
}

/// Add a favorite (idempotent). `NotFound` if the camera doesn't exist.
pub async fn add(db: &SqlitePool, user: Uuid, camera: Uuid) -> Result<(), OrgError> {
    let (huser, hcam) = (user.hyphenated(), camera.hyphenated());
    let now = OffsetDateTime::now_utc();
    let done = sqlx::query!(
        "INSERT OR IGNORE INTO favorites (user_id, camera_id, created_at)
         SELECT ?1, id, ?3 FROM cameras WHERE id = ?2",
        huser,
        hcam,
        now
    )
    .execute(db)
    .await?;
    if done.rows_affected() == 0 {
        let exists: Option<i64> = sqlx::query_scalar!("SELECT 1 FROM cameras WHERE id = ?1", hcam)
            .fetch_optional(db)
            .await?;
        if exists.is_none() {
            return Err(OrgError::NotFound);
        }
    }
    Ok(())
}

/// Remove a favorite (idempotent).
pub async fn remove(db: &SqlitePool, user: Uuid, camera: Uuid) -> Result<(), sqlx::Error> {
    let (huser, hcam) = (user.hyphenated(), camera.hyphenated());
    sqlx::query!(
        "DELETE FROM favorites WHERE user_id = ?1 AND camera_id = ?2",
        huser,
        hcam
    )
    .execute(db)
    .await?;
    Ok(())
}
