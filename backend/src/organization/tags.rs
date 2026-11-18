//! Tags and camera tagging.

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use super::model::Tag;
use super::{is_unique_violation, json_ids, unique, validate_name, OrgError};

struct TagRow {
    id: Hyphenated,
    name: String,
    camera_count: i64,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<TagRow> for Tag {
    fn from(r: TagRow) -> Self {
        Tag {
            id: r.id.into_uuid(),
            name: r.name,
            camera_count: r.camera_count,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn duplicate() -> OrgError {
    OrgError::Duplicate("A tag with this name already exists (names ignore case).".into())
}

pub async fn list(db: &SqlitePool) -> Result<Vec<Tag>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TagRow,
        r#"SELECT t.id AS "id: Hyphenated", t.name,
                  (SELECT COUNT(*) FROM camera_tags ct WHERE ct.tag_id = t.id) AS "camera_count!: i64",
                  t.created_at AS "created_at: OffsetDateTime",
                  t.updated_at AS "updated_at: OffsetDateTime"
             FROM tags t
            ORDER BY t.name COLLATE NOCASE"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Tag::from).collect())
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<Tag>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query_as!(
        TagRow,
        r#"SELECT t.id AS "id: Hyphenated", t.name,
                  (SELECT COUNT(*) FROM camera_tags ct WHERE ct.tag_id = t.id) AS "camera_count!: i64",
                  t.created_at AS "created_at: OffsetDateTime",
                  t.updated_at AS "updated_at: OffsetDateTime"
             FROM tags t
            WHERE t.id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Tag::from))
}

pub async fn create(db: &SqlitePool, name: &str) -> Result<Tag, OrgError> {
    let name = validate_name("tag", name)?;
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    match sqlx::query!(
        "INSERT INTO tags (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        hid,
        name,
        now
    )
    .execute(db)
    .await
    {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(duplicate()),
        Err(e) => return Err(e.into()),
    }
    get(db, id).await?.ok_or(OrgError::NotFound)
}

pub async fn rename(db: &SqlitePool, id: Uuid, name: &str) -> Result<(Tag, Tag), OrgError> {
    let before = get(db, id).await?.ok_or(OrgError::NotFound)?;
    let name = validate_name("tag", name)?;
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    match sqlx::query!(
        "UPDATE tags SET name = ?1, updated_at = ?2 WHERE id = ?3",
        name,
        now,
        hid
    )
    .execute(db)
    .await
    {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(duplicate()),
        Err(e) => return Err(e.into()),
    }
    let after = get(db, id).await?.ok_or(OrgError::NotFound)?;
    Ok((before, after))
}

/// Delete a tag; it's removed from every camera.
pub async fn delete(db: &SqlitePool, id: Uuid) -> Result<Tag, OrgError> {
    let tag = get(db, id).await?.ok_or(OrgError::NotFound)?;
    let hid = id.hyphenated();
    sqlx::query!("DELETE FROM tags WHERE id = ?1", hid)
        .execute(db)
        .await?;
    Ok(tag)
}

/// Cameras with a tag.
pub async fn members(db: &SqlitePool, id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let hid = id.hyphenated();
    let ids = sqlx::query_scalar!(
        r#"SELECT camera_id AS "id: Hyphenated" FROM camera_tags WHERE tag_id = ?1"#,
        hid
    )
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().map(Hyphenated::into_uuid).collect())
}

/// Fail with `Invalid` unless every tag in `ids` exists.
pub async fn ensure_exist(db: &SqlitePool, ids: &[Uuid]) -> Result<(), OrgError> {
    let ids = unique(ids);
    let json = json_ids(&ids);
    let found: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tags WHERE id IN (SELECT value FROM json_each(?1))"#,
        json
    )
    .fetch_one(db)
    .await?;
    if usize::try_from(found).unwrap_or(0) != ids.len() {
        return Err(OrgError::Invalid("some of the tags don't exist".into()));
    }
    Ok(())
}

/// Replace a camera's tags.
pub async fn set_for_camera(db: &SqlitePool, camera: Uuid, tags: &[Uuid]) -> Result<(), OrgError> {
    ensure_exist(db, tags).await?;
    let hcam = camera.hyphenated();
    let json = json_ids(&unique(tags));
    let mut tx = db.begin().await?;
    sqlx::query!("DELETE FROM camera_tags WHERE camera_id = ?1", hcam)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "INSERT INTO camera_tags (camera_id, tag_id) SELECT ?1, value FROM json_each(?2)",
        hcam,
        json
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn add_cameras(
    db: &SqlitePool,
    cameras: &[Uuid],
    tags: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let (c, t) = (json_ids(cameras), json_ids(tags));
    let done = sqlx::query!(
        "INSERT OR IGNORE INTO camera_tags (camera_id, tag_id)
         SELECT c.value, t.value FROM json_each(?1) c, json_each(?2) t",
        c,
        t
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

pub async fn remove_cameras(
    db: &SqlitePool,
    cameras: &[Uuid],
    tags: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let (c, t) = (json_ids(cameras), json_ids(tags));
    let done = sqlx::query!(
        "DELETE FROM camera_tags
          WHERE camera_id IN (SELECT value FROM json_each(?1))
            AND tag_id IN (SELECT value FROM json_each(?2))",
        c,
        t
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}
