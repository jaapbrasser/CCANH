//! Groups, tags and favorites (SPEC §9, ADR 0009).
//!
//! Groups and tags are global and admin-managed; favorites belong to one user
//! and are never shown to others. Groups form a tree stored as an adjacency
//! list; moves that would create a cycle are refused.

pub mod favorites;
pub mod groups;
pub mod model;
pub mod tags;

use std::collections::HashMap;

use sqlx::SqlitePool;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Duplicate(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub(crate) fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Longest group or tag name, in characters.
pub const MAX_NAME_LEN: usize = 64;

/// Trim and check a group or tag name.
pub fn validate_name(what: &str, name: &str) -> Result<String, OrgError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
    {
        return Err(OrgError::Invalid(format!(
            "{what} name must be 1–{MAX_NAME_LEN} characters, without control characters"
        )));
    }
    Ok(name.to_owned())
}

fn json_ids(ids: &[Uuid]) -> String {
    serde_json::to_string(
        &ids.iter()
            .map(|u| u.hyphenated().to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into())
}

/// Dedup while keeping order.
pub(crate) fn unique(ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// Groups and tags of each camera in `camera_ids`, sorted by name.
pub async fn memberships(
    db: &SqlitePool,
    camera_ids: &[Uuid],
) -> Result<HashMap<Uuid, (Vec<GroupRef>, Vec<TagRef>)>, sqlx::Error> {
    let mut out: HashMap<Uuid, (Vec<GroupRef>, Vec<TagRef>)> = HashMap::new();
    if camera_ids.is_empty() {
        return Ok(out);
    }
    let ids = json_ids(camera_ids);
    let groups = sqlx::query!(
        r#"SELECT cg.camera_id AS "camera_id: Hyphenated", g.id AS "id: Hyphenated", g.name
             FROM camera_groups cg JOIN groups g ON g.id = cg.group_id
            WHERE cg.camera_id IN (SELECT value FROM json_each(?1))
            ORDER BY g.name COLLATE NOCASE"#,
        ids
    )
    .fetch_all(db)
    .await?;
    for r in groups {
        out.entry(r.camera_id.into_uuid())
            .or_default()
            .0
            .push(GroupRef {
                id: r.id.into_uuid(),
                name: r.name,
            });
    }
    let tags = sqlx::query!(
        r#"SELECT ct.camera_id AS "camera_id: Hyphenated", t.id AS "id: Hyphenated", t.name
             FROM camera_tags ct JOIN tags t ON t.id = ct.tag_id
            WHERE ct.camera_id IN (SELECT value FROM json_each(?1))
            ORDER BY t.name COLLATE NOCASE"#,
        ids
    )
    .fetch_all(db)
    .await?;
    for r in tags {
        out.entry(r.camera_id.into_uuid())
            .or_default()
            .1
            .push(TagRef {
                id: r.id.into_uuid(),
                name: r.name,
            });
    }
    Ok(out)
}

/// Fail with `Invalid` unless every camera in `ids` exists.
pub async fn ensure_cameras_exist(db: &SqlitePool, ids: &[Uuid]) -> Result<(), OrgError> {
    let ids = unique(ids);
    let json = json_ids(&ids);
    let found: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM cameras WHERE id IN (SELECT value FROM json_each(?1))"#,
        json
    )
    .fetch_one(db)
    .await?;
    if usize::try_from(found).unwrap_or(0) != ids.len() {
        return Err(OrgError::Invalid("some of the cameras don't exist".into()));
    }
    Ok(())
}
