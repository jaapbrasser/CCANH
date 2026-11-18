//! Group hierarchy and camera membership.

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

use super::model::{DeleteMode, Group, GroupPatch, NewGroup};
use super::{is_unique_violation, json_ids, unique, validate_name, OrgError};

struct GroupRow {
    id: Hyphenated,
    name: String,
    parent_id: Option<Hyphenated>,
    camera_count: i64,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<GroupRow> for Group {
    fn from(r: GroupRow) -> Self {
        Group {
            id: r.id.into_uuid(),
            name: r.name,
            parent_id: r.parent_id.map(Hyphenated::into_uuid),
            camera_count: r.camera_count,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn duplicate() -> OrgError {
    OrgError::Duplicate("A group with this name already exists at that level.".into())
}

/// Every group, ordered by name (the client builds the tree).
pub async fn list(db: &SqlitePool) -> Result<Vec<Group>, sqlx::Error> {
    let rows = sqlx::query_as!(
        GroupRow,
        r#"SELECT g.id AS "id: Hyphenated", g.name, g.parent_id AS "parent_id: Hyphenated",
                  (SELECT COUNT(*) FROM camera_groups cg WHERE cg.group_id = g.id) AS "camera_count!: i64",
                  g.created_at AS "created_at: OffsetDateTime",
                  g.updated_at AS "updated_at: OffsetDateTime"
             FROM groups g
            ORDER BY g.name COLLATE NOCASE, g.id"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Group::from).collect())
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<Group>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query_as!(
        GroupRow,
        r#"SELECT g.id AS "id: Hyphenated", g.name, g.parent_id AS "parent_id: Hyphenated",
                  (SELECT COUNT(*) FROM camera_groups cg WHERE cg.group_id = g.id) AS "camera_count!: i64",
                  g.created_at AS "created_at: OffsetDateTime",
                  g.updated_at AS "updated_at: OffsetDateTime"
             FROM groups g
            WHERE g.id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Group::from))
}

/// `id` and all its descendants.
pub async fn subtree(db: &SqlitePool, id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let hid = id.hyphenated();
    let ids = sqlx::query_scalar!(
        r#"WITH RECURSIVE sub(id) AS (
               SELECT ?1
               UNION
               SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id
           )
           SELECT id AS "id!: Hyphenated" FROM sub"#,
        hid
    )
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().map(Hyphenated::into_uuid).collect())
}

async fn require_parent(db: &SqlitePool, parent: Option<Uuid>) -> Result<(), OrgError> {
    if let Some(p) = parent {
        if get(db, p).await?.is_none() {
            return Err(OrgError::Invalid("the parent group doesn't exist".into()));
        }
    }
    Ok(())
}

pub async fn create(db: &SqlitePool, g: &NewGroup) -> Result<Group, OrgError> {
    let name = validate_name("group", &g.name)?;
    require_parent(db, g.parent_id).await?;
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let parent = g.parent_id.map(Uuid::hyphenated);
    let now = OffsetDateTime::now_utc();
    match sqlx::query!(
        "INSERT INTO groups (id, name, parent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        hid,
        name,
        parent,
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

/// Rename and/or move. Moving under itself or a descendant is refused.
pub async fn update(db: &SqlitePool, id: Uuid, p: &GroupPatch) -> Result<(Group, Group), OrgError> {
    let before = get(db, id).await?.ok_or(OrgError::NotFound)?;
    let name = match &p.name {
        Some(n) => validate_name("group", n)?,
        None => before.name.clone(),
    };
    let parent = match p.parent_id {
        Some(new_parent) => {
            require_parent(db, new_parent).await?;
            if let Some(np) = new_parent {
                if subtree(db, id).await?.contains(&np) {
                    return Err(OrgError::Invalid(
                        "a group can't be moved into itself or one of its subgroups".into(),
                    ));
                }
            }
            new_parent
        }
        None => before.parent_id,
    };
    let hid = id.hyphenated();
    let hparent = parent.map(Uuid::hyphenated);
    let now = OffsetDateTime::now_utc();
    match sqlx::query!(
        "UPDATE groups SET name = ?1, parent_id = ?2, updated_at = ?3 WHERE id = ?4",
        name,
        hparent,
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

/// Delete a group, handling its subgroups and cameras per `mode`.
pub async fn delete(db: &SqlitePool, id: Uuid, mode: DeleteMode) -> Result<Group, OrgError> {
    let group = get(db, id).await?.ok_or(OrgError::NotFound)?;
    let hid = id.hyphenated();
    let new_parent = match mode {
        DeleteMode::MoveUp => group.parent_id,
        DeleteMode::Detach => None,
    };
    let hparent = new_parent.map(Uuid::hyphenated);
    let now = OffsetDateTime::now_utc();

    let mut tx = db.begin().await?;
    match sqlx::query!(
        "UPDATE groups SET parent_id = ?1, updated_at = ?2 WHERE parent_id = ?3",
        hparent,
        now,
        hid
    )
    .execute(&mut *tx)
    .await
    {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(OrgError::Duplicate(
                "A subgroup has the same name as a group at the level it would move to. Rename it first."
                    .into(),
            ))
        }
        Err(e) => return Err(e.into()),
    }
    if let (DeleteMode::MoveUp, Some(parent)) = (mode, hparent) {
        sqlx::query!(
            "INSERT OR IGNORE INTO camera_groups (camera_id, group_id)
             SELECT camera_id, ?1 FROM camera_groups WHERE group_id = ?2",
            parent,
            hid
        )
        .execute(&mut *tx)
        .await?;
    }
    // Memberships of the deleted group go with it (ON DELETE CASCADE).
    sqlx::query!("DELETE FROM groups WHERE id = ?1", hid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(group)
}

/// Cameras directly in a group (not via subgroups).
pub async fn direct_members(db: &SqlitePool, id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let hid = id.hyphenated();
    let ids = sqlx::query_scalar!(
        r#"SELECT camera_id AS "id: Hyphenated" FROM camera_groups WHERE group_id = ?1"#,
        hid
    )
    .fetch_all(db)
    .await?;
    Ok(ids.into_iter().map(Hyphenated::into_uuid).collect())
}

/// Fail with `Invalid` unless every group in `ids` exists.
pub async fn ensure_exist(db: &SqlitePool, ids: &[Uuid]) -> Result<(), OrgError> {
    let ids = unique(ids);
    let json = json_ids(&ids);
    let found: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM groups WHERE id IN (SELECT value FROM json_each(?1))"#,
        json
    )
    .fetch_one(db)
    .await?;
    if usize::try_from(found).unwrap_or(0) != ids.len() {
        return Err(OrgError::Invalid("some of the groups don't exist".into()));
    }
    Ok(())
}

/// Replace a camera's groups.
pub async fn set_for_camera(
    db: &SqlitePool,
    camera: Uuid,
    groups: &[Uuid],
) -> Result<(), OrgError> {
    ensure_exist(db, groups).await?;
    let hcam = camera.hyphenated();
    let json = json_ids(&unique(groups));
    let mut tx = db.begin().await?;
    sqlx::query!("DELETE FROM camera_groups WHERE camera_id = ?1", hcam)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "INSERT INTO camera_groups (camera_id, group_id) SELECT ?1, value FROM json_each(?2)",
        hcam,
        json
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Add every camera to every group (existing memberships are kept).
pub async fn add_cameras(
    db: &SqlitePool,
    cameras: &[Uuid],
    groups: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let (c, g) = (json_ids(cameras), json_ids(groups));
    let done = sqlx::query!(
        "INSERT OR IGNORE INTO camera_groups (camera_id, group_id)
         SELECT c.value, g.value FROM json_each(?1) c, json_each(?2) g",
        c,
        g
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

/// Remove every camera from every group.
pub async fn remove_cameras(
    db: &SqlitePool,
    cameras: &[Uuid],
    groups: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let (c, g) = (json_ids(cameras), json_ids(groups));
    let done = sqlx::query!(
        "DELETE FROM camera_groups
          WHERE camera_id IN (SELECT value FROM json_each(?1))
            AND group_id IN (SELECT value FROM json_each(?2))",
        c,
        g
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}
