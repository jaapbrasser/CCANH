//! Per-user dashboards (SPEC §8, ADR 0005): a named rows × cols grid whose
//! tiles each hold one camera or nothing. Private to their owner.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

/// Largest grid side (SPEC §8.2: custom grids up to 8×8).
pub const MAX_SIDE: u8 = 8;
/// Most dashboards one user may have.
pub const MAX_PER_USER: i64 = 50;
pub const MAX_NAME_LEN: usize = 64;

/// One occupied tile. Positions count row by row from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Tile {
    pub position: u16,
    pub camera_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Dashboard {
    pub id: Uuid,
    pub name: String,
    pub rows: u8,
    pub cols: u8,
    /// Occupied tiles only, by position. Cameras deleted since are left out.
    pub tiles: Vec<Tile>,
    /// The dashboard shown first. Each user has exactly one while they have any.
    pub is_default: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

fn default_side() -> u8 {
    2
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewDashboard {
    pub name: String,
    /// 1–8, default 2.
    #[serde(default = "default_side")]
    pub rows: u8,
    /// 1–8, default 2.
    #[serde(default = "default_side")]
    pub cols: u8,
    #[serde(default)]
    pub tiles: Vec<Tile>,
}

/// Omitted fields are unchanged. Shrinking the grid without sending `tiles`
/// drops tiles that no longer fit.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct DashboardPatch {
    pub name: Option<String>,
    pub rows: Option<u8>,
    pub cols: Option<u8>,
    /// Replaces all tiles.
    pub tiles: Option<Vec<Tile>>,
    /// `true` makes this the default dashboard (the previous one stops being it).
    pub make_default: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum DashboardError {
    #[error("dashboard not found")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

fn invalid(m: impl Into<String>) -> DashboardError {
    DashboardError::Invalid(m.into())
}

fn validate_name(name: &str) -> Result<String, DashboardError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN || name.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "name must be 1–{MAX_NAME_LEN} characters, without control characters"
        )));
    }
    Ok(name.to_owned())
}

fn validate_side(what: &str, v: u8) -> Result<u8, DashboardError> {
    if !(1..=MAX_SIDE).contains(&v) {
        return Err(invalid(format!("{what} must be between 1 and {MAX_SIDE}")));
    }
    Ok(v)
}

/// Check positions and cameras; returns tiles sorted by position.
async fn validate_tiles(
    db: &SqlitePool,
    rows: u8,
    cols: u8,
    tiles: &[Tile],
) -> Result<Vec<Tile>, DashboardError> {
    let cells = u16::from(rows) * u16::from(cols);
    let mut positions = HashSet::new();
    let mut cameras = HashSet::new();
    for t in tiles {
        if t.position >= cells {
            return Err(invalid(format!(
                "tile position {} is outside the {rows}×{cols} grid",
                t.position
            )));
        }
        if !positions.insert(t.position) {
            return Err(invalid(format!("two tiles share position {}", t.position)));
        }
        if !cameras.insert(t.camera_id) {
            return Err(invalid("a camera can appear only once per dashboard"));
        }
    }
    let existing = camera_ids(db).await?;
    if cameras.iter().any(|c| !existing.contains(c)) {
        return Err(invalid("some of the cameras don't exist"));
    }
    let mut sorted = tiles.to_vec();
    sorted.sort_by_key(|t| t.position);
    Ok(sorted)
}

async fn camera_ids(db: &SqlitePool) -> Result<HashSet<Uuid>, sqlx::Error> {
    let ids = sqlx::query_scalar!(r#"SELECT id AS "id: Hyphenated" FROM cameras"#)
        .fetch_all(db)
        .await?;
    Ok(ids.into_iter().map(Hyphenated::into_uuid).collect())
}

struct Row {
    id: Hyphenated,
    name: String,
    rows: i64,
    cols: i64,
    tiles: String,
    is_default: bool,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

fn to_dashboard(r: Row, cameras: &HashSet<Uuid>) -> Dashboard {
    let rows = u8::try_from(r.rows).unwrap_or(1);
    let cols = u8::try_from(r.cols).unwrap_or(1);
    let cells = u16::from(rows) * u16::from(cols);
    let tiles = serde_json::from_str::<Vec<Tile>>(&r.tiles)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.position < cells && cameras.contains(&t.camera_id))
        .collect();
    Dashboard {
        id: r.id.into_uuid(),
        name: r.name,
        rows,
        cols,
        tiles,
        is_default: r.is_default,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

/// The user's dashboards, oldest first.
pub async fn list(db: &SqlitePool, user: Uuid) -> Result<Vec<Dashboard>, sqlx::Error> {
    let huser = user.hyphenated();
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT id AS "id: Hyphenated", name, rows, cols, tiles,
                  is_default AS "is_default: bool",
                  created_at AS "created_at: OffsetDateTime",
                  updated_at AS "updated_at: OffsetDateTime"
             FROM dashboards WHERE user_id = ?1
            ORDER BY created_at, id"#,
        huser
    )
    .fetch_all(db)
    .await?;
    let cameras = camera_ids(db).await?;
    Ok(rows
        .into_iter()
        .map(|r| to_dashboard(r, &cameras))
        .collect())
}

/// One of the user's dashboards (`None` if missing or someone else's).
pub async fn get(db: &SqlitePool, user: Uuid, id: Uuid) -> Result<Option<Dashboard>, sqlx::Error> {
    let (huser, hid) = (user.hyphenated(), id.hyphenated());
    let row = sqlx::query_as!(
        Row,
        r#"SELECT id AS "id: Hyphenated", name, rows, cols, tiles,
                  is_default AS "is_default: bool",
                  created_at AS "created_at: OffsetDateTime",
                  updated_at AS "updated_at: OffsetDateTime"
             FROM dashboards WHERE id = ?1 AND user_id = ?2"#,
        hid,
        huser
    )
    .fetch_optional(db)
    .await?;
    match row {
        Some(r) => Ok(Some(to_dashboard(r, &camera_ids(db).await?))),
        None => Ok(None),
    }
}

pub async fn create(
    db: &SqlitePool,
    user: Uuid,
    d: &NewDashboard,
) -> Result<Dashboard, DashboardError> {
    let name = validate_name(&d.name)?;
    let rows = validate_side("rows", d.rows)?;
    let cols = validate_side("cols", d.cols)?;
    let tiles = validate_tiles(db, rows, cols, &d.tiles).await?;
    let huser = user.hyphenated();
    let count: i64 = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM dashboards WHERE user_id = ?1"#,
        huser
    )
    .fetch_one(db)
    .await?;
    if count >= MAX_PER_USER {
        return Err(invalid(format!(
            "you can have at most {MAX_PER_USER} dashboards"
        )));
    }
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let json = serde_json::to_string(&tiles).map_err(|e| invalid(e.to_string()))?;
    let is_default = count == 0;
    let now = OffsetDateTime::now_utc();
    let (r, c) = (i64::from(rows), i64::from(cols));
    sqlx::query!(
        "INSERT INTO dashboards (id, user_id, name, rows, cols, tiles, is_default, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        hid,
        huser,
        name,
        r,
        c,
        json,
        is_default,
        now
    )
    .execute(db)
    .await?;
    get(db, user, id).await?.ok_or(DashboardError::NotFound)
}

pub async fn update(
    db: &SqlitePool,
    user: Uuid,
    id: Uuid,
    p: &DashboardPatch,
) -> Result<Dashboard, DashboardError> {
    let current = get(db, user, id).await?.ok_or(DashboardError::NotFound)?;
    let name = match &p.name {
        Some(n) => validate_name(n)?,
        None => current.name.clone(),
    };
    let rows = validate_side("rows", p.rows.unwrap_or(current.rows))?;
    let cols = validate_side("cols", p.cols.unwrap_or(current.cols))?;
    let tiles = match &p.tiles {
        Some(t) => validate_tiles(db, rows, cols, t).await?,
        None => {
            let cells = u16::from(rows) * u16::from(cols);
            current
                .tiles
                .iter()
                .copied()
                .filter(|t| t.position < cells)
                .collect()
        }
    };
    let json = serde_json::to_string(&tiles).map_err(|e| invalid(e.to_string()))?;
    let (hid, huser) = (id.hyphenated(), user.hyphenated());
    let now = OffsetDateTime::now_utc();
    let (r, c) = (i64::from(rows), i64::from(cols));
    let mut tx = db.begin().await?;
    if p.make_default == Some(true) {
        sqlx::query!(
            "UPDATE dashboards SET is_default = 0 WHERE user_id = ?1 AND id != ?2",
            huser,
            hid
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("UPDATE dashboards SET is_default = 1 WHERE id = ?1", hid)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query!(
        "UPDATE dashboards SET name = ?1, rows = ?2, cols = ?3, tiles = ?4, updated_at = ?5
          WHERE id = ?6 AND user_id = ?7",
        name,
        r,
        c,
        json,
        now,
        hid,
        huser
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get(db, user, id).await?.ok_or(DashboardError::NotFound)
}

/// Delete a dashboard. If it was the default, the oldest remaining one
/// becomes the default.
pub async fn delete(db: &SqlitePool, user: Uuid, id: Uuid) -> Result<(), DashboardError> {
    let (hid, huser) = (id.hyphenated(), user.hyphenated());
    let mut tx = db.begin().await?;
    let done = sqlx::query!(
        "DELETE FROM dashboards WHERE id = ?1 AND user_id = ?2",
        hid,
        huser
    )
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(DashboardError::NotFound);
    }
    sqlx::query!(
        "UPDATE dashboards SET is_default = 1
          WHERE id = (SELECT id FROM dashboards WHERE user_id = ?1 ORDER BY created_at, id LIMIT 1)
            AND NOT EXISTS (SELECT 1 FROM dashboards WHERE user_id = ?1 AND is_default = 1)",
        huser
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
