//! Camera registry: persistence of cameras, their identity and their status.

pub mod model;

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

pub use model::*;

use crate::providers::{CameraCapabilities, DeviceInfo, ErrorCategory, StreamProfile};

#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("camera not found")]
    NotFound,
    #[error("a camera with this host and port already exists")]
    Duplicate,
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

fn port_u16(port: i64) -> u16 {
    // The column has CHECK (port BETWEEN 1 AND 65535).
    u16::try_from(port).unwrap_or(0)
}

fn opt_u32(v: Option<i64>) -> Option<u32> {
    v.and_then(|v| u32::try_from(v).ok())
}

/// Columns shared by list and detail queries.
struct SummaryRow {
    id: Hyphenated,
    name: String,
    host: String,
    port: i64,
    scheme: Scheme,
    manufacturer: Option<String>,
    model: Option<String>,
    firmware_version: Option<String>,
    monitoring_enabled: bool,
    state: CameraState,
    last_success_at: Option<OffsetDateTime>,
    last_failure_at: Option<OffsetDateTime>,
    last_change_at: Option<OffsetDateTime>,
    last_checked_at: Option<OffsetDateTime>,
    response_time_ms: Option<i64>,
    consecutive_failures: i64,
    error_category: Option<ErrorCategory>,
    error_message: Option<String>,
}

impl From<SummaryRow> for CameraSummary {
    fn from(r: SummaryRow) -> Self {
        CameraSummary {
            id: r.id.into_uuid(),
            name: r.name,
            host: r.host,
            port: port_u16(r.port),
            scheme: r.scheme,
            manufacturer: r.manufacturer,
            model: r.model,
            firmware_version: r.firmware_version,
            monitoring_enabled: r.monitoring_enabled,
            status: CameraStatusView {
                state: r.state,
                last_success_at: r.last_success_at,
                last_failure_at: r.last_failure_at,
                last_change_at: r.last_change_at,
                last_checked_at: r.last_checked_at,
                response_time_ms: opt_u32(r.response_time_ms),
                consecutive_failures: u32::try_from(r.consecutive_failures).unwrap_or(u32::MAX),
                error_category: r.error_category,
                error_message: r.error_message,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CameraSort {
    #[default]
    Name,
    Host,
    Model,
    State,
    LastSuccessAt,
    LastChangeAt,
}

impl CameraSort {
    fn as_str(self) -> &'static str {
        match self {
            CameraSort::Name => "name",
            CameraSort::Host => "host",
            CameraSort::Model => "model",
            CameraSort::State => "state",
            CameraSort::LastSuccessAt => "last_success_at",
            CameraSort::LastChangeAt => "last_change_at",
        }
    }
}

pub struct ListParams<'a> {
    pub search: Option<&'a str>,
    pub state: Option<CameraState>,
    pub sort: CameraSort,
    pub descending: bool,
    pub limit: i64,
    pub offset: i64,
}

fn like_pattern(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

pub async fn list(
    db: &SqlitePool,
    p: ListParams<'_>,
) -> Result<(Vec<CameraSummary>, i64), sqlx::Error> {
    let pattern = p.search.map(like_pattern);
    let state = p.state;
    let sort = p.sort.as_str();
    let order = if p.descending { "desc" } else { "asc" };

    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64"
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE (?1 IS NULL OR c.name LIKE ?1 ESCAPE '\' OR c.host LIKE ?1 ESCAPE '\'
                   OR c.model LIKE ?1 ESCAPE '\')
              AND (?2 IS NULL OR s.state = ?2)"#,
        pattern,
        state
    )
    .fetch_one(db)
    .await?;

    // ORDER BY can't be a bind parameter, so each sort/order pair is a CASE arm.
    let rows = sqlx::query_as!(
        SummaryRow,
        r#"SELECT c.id AS "id: Hyphenated", c.name, c.host, c.port, c.scheme AS "scheme: Scheme",
                  c.manufacturer, c.model, c.firmware_version,
                  c.monitoring_enabled AS "monitoring_enabled: bool",
                  s.state AS "state: CameraState",
                  s.last_success_at AS "last_success_at: OffsetDateTime",
                  s.last_failure_at AS "last_failure_at: OffsetDateTime",
                  s.last_change_at AS "last_change_at: OffsetDateTime",
                  s.last_checked_at AS "last_checked_at: OffsetDateTime",
                  s.response_time_ms, s.consecutive_failures,
                  s.error_category AS "error_category: ErrorCategory", s.error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE (?1 IS NULL OR c.name LIKE ?1 ESCAPE '\' OR c.host LIKE ?1 ESCAPE '\'
                   OR c.model LIKE ?1 ESCAPE '\')
              AND (?2 IS NULL OR s.state = ?2)
            ORDER BY
              CASE WHEN ?3 = 'name'  AND ?4 = 'asc'  THEN c.name  COLLATE NOCASE END ASC,
              CASE WHEN ?3 = 'name'  AND ?4 = 'desc' THEN c.name  COLLATE NOCASE END DESC,
              CASE WHEN ?3 = 'host'  AND ?4 = 'asc'  THEN c.host  END ASC,
              CASE WHEN ?3 = 'host'  AND ?4 = 'desc' THEN c.host  END DESC,
              CASE WHEN ?3 = 'model' AND ?4 = 'asc'  THEN c.model END ASC,
              CASE WHEN ?3 = 'model' AND ?4 = 'desc' THEN c.model END DESC,
              CASE WHEN ?3 = 'state' AND ?4 = 'asc'  THEN s.state END ASC,
              CASE WHEN ?3 = 'state' AND ?4 = 'desc' THEN s.state END DESC,
              CASE WHEN ?3 = 'last_success_at' AND ?4 = 'asc'  THEN s.last_success_at END ASC,
              CASE WHEN ?3 = 'last_success_at' AND ?4 = 'desc' THEN s.last_success_at END DESC,
              CASE WHEN ?3 = 'last_change_at'  AND ?4 = 'asc'  THEN s.last_change_at  END ASC,
              CASE WHEN ?3 = 'last_change_at'  AND ?4 = 'desc' THEN s.last_change_at  END DESC,
              c.name COLLATE NOCASE ASC, c.id ASC
            LIMIT ?5 OFFSET ?6"#,
        pattern,
        state,
        sort,
        order,
        p.limit,
        p.offset
    )
    .fetch_all(db)
    .await?;

    Ok((rows.into_iter().map(CameraSummary::from).collect(), total))
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<CameraDetail>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query!(
        r#"SELECT c.id AS "id: Hyphenated", c.name, c.host, c.port, c.scheme AS "scheme: Scheme",
                  c.manufacturer, c.model, c.firmware_version,
                  c.monitoring_enabled AS "monitoring_enabled: bool",
                  c.tls_mode AS "tls_mode: TlsMode", c.tls_pinned_sha256,
                  c.serial_number, c.mac_address, c.device_name, c.device_hostname,
                  c.capabilities, c.stream_profiles, c.network,
                  c.identity_updated_at AS "identity_updated_at: OffsetDateTime",
                  c.created_at AS "created_at: OffsetDateTime",
                  c.updated_at AS "updated_at: OffsetDateTime",
                  s.state AS "state: CameraState",
                  s.last_success_at AS "last_success_at: OffsetDateTime",
                  s.last_failure_at AS "last_failure_at: OffsetDateTime",
                  s.last_change_at AS "last_change_at: OffsetDateTime",
                  s.last_checked_at AS "last_checked_at: OffsetDateTime",
                  s.response_time_ms, s.consecutive_failures,
                  s.error_category AS "error_category: ErrorCategory", s.error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE c.id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| CameraDetail {
        summary: CameraSummary::from(SummaryRow {
            id: r.id,
            name: r.name,
            host: r.host,
            port: r.port,
            scheme: r.scheme,
            manufacturer: r.manufacturer,
            model: r.model,
            firmware_version: r.firmware_version,
            monitoring_enabled: r.monitoring_enabled,
            state: r.state,
            last_success_at: r.last_success_at,
            last_failure_at: r.last_failure_at,
            last_change_at: r.last_change_at,
            last_checked_at: r.last_checked_at,
            response_time_ms: r.response_time_ms,
            consecutive_failures: r.consecutive_failures,
            error_category: r.error_category,
            error_message: r.error_message,
        }),
        tls_mode: r.tls_mode,
        tls_pinned_sha256: r.tls_pinned_sha256,
        serial_number: r.serial_number,
        mac_address: r.mac_address,
        device_name: r.device_name,
        device_hostname: r.device_hostname,
        network: r.network.and_then(|n| serde_json::from_str(&n).ok()),
        capabilities: r
            .capabilities
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default(),
        stream_profiles: r
            .stream_profiles
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        identity_updated_at: r.identity_updated_at,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

/// Validated, complete settings for inserting or updating a camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSettings {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    pub tls_mode: TlsMode,
    pub tls_pinned_sha256: Option<String>,
    pub monitoring_enabled: bool,
}

impl CameraSettings {
    pub fn from_new(n: NewCamera) -> Result<Self, ValidationError> {
        let scheme = n.scheme;
        Self::validated(Self {
            name: n.name,
            host: n.host,
            port: n.port.unwrap_or_else(|| scheme.default_port()),
            scheme,
            tls_mode: n.tls_mode,
            tls_pinned_sha256: n.tls_pinned_sha256,
            monitoring_enabled: n.monitoring_enabled,
        })
    }

    /// Apply a patch to the current settings.
    pub fn patched(current: &CameraDetail, p: CameraPatch) -> Result<Self, ValidationError> {
        let s = &current.summary;
        let tls_mode = p.tls_mode.unwrap_or(current.tls_mode);
        let pin = match (p.tls_pinned_sha256, tls_mode) {
            (Some(pin), _) => Some(pin),
            (None, TlsMode::Pinned) => current.tls_pinned_sha256.clone(),
            (None, _) => None,
        };
        Self::validated(Self {
            name: p.name.unwrap_or_else(|| s.name.clone()),
            host: p.host.unwrap_or_else(|| s.host.clone()),
            port: p.port.unwrap_or(s.port),
            scheme: p.scheme.unwrap_or(s.scheme),
            tls_mode,
            tls_pinned_sha256: pin,
            monitoring_enabled: p.monitoring_enabled.unwrap_or(s.monitoring_enabled),
        })
    }

    fn validated(mut s: Self) -> Result<Self, ValidationError> {
        s.name = validate_name(&s.name)?;
        s.host = validate_host(&s.host)?;
        s.port = validate_port(s.port)?;
        s.tls_pinned_sha256 = match (s.tls_mode, s.tls_pinned_sha256.as_deref()) {
            (TlsMode::Pinned, None) => return Err(ValidationError::PinMissing),
            (TlsMode::Pinned, Some(fp)) => Some(normalize_fingerprint(fp)?),
            (_, _) => None,
        };
        Ok(s)
    }

    /// Whether a change points the record at a (potentially) different device.
    fn targets_differ(&self, current: &CameraDetail) -> bool {
        let s = &current.summary;
        self.host != s.host || self.port != s.port || self.scheme != s.scheme
    }
}

pub async fn create(db: &SqlitePool, s: &CameraSettings) -> Result<CameraDetail, CameraError> {
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let port = i64::from(s.port);
    let mut tx = db.begin().await?;
    let inserted = sqlx::query!(
        "INSERT INTO cameras (id, name, host, port, scheme, tls_mode, tls_pinned_sha256,
                              monitoring_enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        hid,
        s.name,
        s.host,
        port,
        s.scheme,
        s.tls_mode,
        s.tls_pinned_sha256,
        s.monitoring_enabled,
        now
    )
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(CameraError::Duplicate),
        Err(e) => return Err(e.into()),
    }
    sqlx::query!("INSERT INTO camera_status (camera_id) VALUES (?1)", hid)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    get(db, id).await?.ok_or(CameraError::NotFound)
}

/// Update settings. Pointing the camera at a different host/port/scheme clears
/// its identity and resets its status, since it may be a different device.
pub async fn update(
    db: &SqlitePool,
    id: Uuid,
    patch: CameraPatch,
) -> Result<(CameraDetail, CameraDetail), CameraError> {
    let before = get(db, id).await?.ok_or(CameraError::NotFound)?;
    let s = CameraSettings::patched(&before, patch)?;
    let reset = s.targets_differ(&before);
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let port = i64::from(s.port);

    let mut tx = db.begin().await?;
    let updated = sqlx::query!(
        "UPDATE cameras
            SET name = ?2, host = ?3, port = ?4, scheme = ?5, tls_mode = ?6,
                tls_pinned_sha256 = ?7, monitoring_enabled = ?8, updated_at = ?9
          WHERE id = ?1",
        hid,
        s.name,
        s.host,
        port,
        s.scheme,
        s.tls_mode,
        s.tls_pinned_sha256,
        s.monitoring_enabled,
        now
    )
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(r) if r.rows_affected() == 0 => return Err(CameraError::NotFound),
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(CameraError::Duplicate),
        Err(e) => return Err(e.into()),
    }
    if reset {
        sqlx::query!(
            "UPDATE cameras SET manufacturer = NULL, model = NULL, firmware_version = NULL,
                    serial_number = NULL, mac_address = NULL, device_name = NULL,
                    device_hostname = NULL, capabilities = NULL, stream_profiles = NULL,
                    network = NULL, identity_updated_at = NULL
              WHERE id = ?1",
            hid
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE camera_status SET state = 'unknown', last_change_at = NULL,
                    response_time_ms = NULL, consecutive_failures = 0,
                    error_category = NULL, error_message = NULL
              WHERE camera_id = ?1",
            hid
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let after = get(db, id).await?.ok_or(CameraError::NotFound)?;
    Ok((before, after))
}

pub async fn delete(db: &SqlitePool, id: Uuid) -> Result<(), CameraError> {
    let hid = id.hyphenated();
    let result = sqlx::query!("DELETE FROM cameras WHERE id = ?1", hid)
        .execute(db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(CameraError::NotFound);
    }
    Ok(())
}

/// What the status monitor needs for each monitored camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorTarget {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    pub updated_at: OffsetDateTime,
    pub has_identity: bool,
    pub status: CameraStatusView,
}

pub async fn monitor_targets(db: &SqlitePool) -> Result<Vec<MonitorTarget>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT c.id AS "id: Hyphenated", c.host, c.port, c.scheme AS "scheme: Scheme",
                  c.updated_at AS "updated_at: OffsetDateTime",
                  (c.identity_updated_at IS NOT NULL) AS "has_identity!: bool",
                  s.state AS "state: CameraState",
                  s.last_success_at AS "last_success_at: OffsetDateTime",
                  s.last_failure_at AS "last_failure_at: OffsetDateTime",
                  s.last_change_at AS "last_change_at: OffsetDateTime",
                  s.last_checked_at AS "last_checked_at: OffsetDateTime",
                  s.response_time_ms, s.consecutive_failures,
                  s.error_category AS "error_category: ErrorCategory", s.error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE c.monitoring_enabled = 1"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MonitorTarget {
            id: r.id.into_uuid(),
            host: r.host,
            port: port_u16(r.port),
            scheme: r.scheme,
            updated_at: r.updated_at,
            has_identity: r.has_identity,
            status: CameraStatusView {
                state: r.state,
                last_success_at: r.last_success_at,
                last_failure_at: r.last_failure_at,
                last_change_at: r.last_change_at,
                last_checked_at: r.last_checked_at,
                response_time_ms: opt_u32(r.response_time_ms),
                consecutive_failures: u32::try_from(r.consecutive_failures).unwrap_or(u32::MAX),
                error_category: r.error_category,
                error_message: r.error_message,
            },
        })
        .collect())
}

/// Persist the current status, and a history row when `transition` is given.
pub async fn write_status(
    db: &SqlitePool,
    id: Uuid,
    status: &CameraStatusView,
    transition: Option<(CameraState, CameraState)>,
) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let rt = status.response_time_ms.map(i64::from);
    let failures = i64::from(status.consecutive_failures);
    let mut tx = db.begin().await?;
    sqlx::query!(
        "UPDATE camera_status
            SET state = ?2, last_success_at = ?3, last_failure_at = ?4, last_change_at = ?5,
                last_checked_at = ?6, response_time_ms = ?7, consecutive_failures = ?8,
                error_category = ?9, error_message = ?10
          WHERE camera_id = ?1",
        hid,
        status.state,
        status.last_success_at,
        status.last_failure_at,
        status.last_change_at,
        status.last_checked_at,
        rt,
        failures,
        status.error_category,
        status.error_message
    )
    .execute(&mut *tx)
    .await?;
    if let Some((from, to)) = transition {
        let at = status
            .last_change_at
            .unwrap_or_else(OffsetDateTime::now_utc);
        sqlx::query!(
            "INSERT INTO camera_status_history (camera_id, occurred_at, from_state, to_state, error_category)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            hid,
            at,
            from,
            to,
            status.error_category
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Store identity, capabilities and stream profiles fetched from the provider.
/// `None` for capabilities/profiles keeps what's stored.
pub async fn save_identity(
    db: &SqlitePool,
    id: Uuid,
    device: &DeviceInfo,
    capabilities: Option<&CameraCapabilities>,
    profiles: Option<&[StreamProfile]>,
) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let network = device
        .network
        .as_ref()
        .and_then(|n| serde_json::to_string(n).ok());
    let caps = capabilities.and_then(|c| serde_json::to_string(&c.items).ok());
    let profiles = profiles.and_then(|p| serde_json::to_string(p).ok());
    sqlx::query!(
        "UPDATE cameras
            SET manufacturer = ?2, model = ?3, firmware_version = ?4, serial_number = ?5,
                mac_address = ?6, device_name = ?7, device_hostname = ?8, network = ?9,
                capabilities = COALESCE(?10, capabilities),
                stream_profiles = COALESCE(?11, stream_profiles),
                identity_updated_at = ?12
          WHERE id = ?1",
        hid,
        device.manufacturer,
        device.model,
        device.firmware_version,
        device.serial_number,
        device.mac_address,
        device.device_name,
        device.hostname,
        network,
        caps,
        profiles,
        now
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn history(
    db: &SqlitePool,
    id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<(Vec<StatusTransition>, i64), sqlx::Error> {
    let hid = id.hyphenated();
    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM camera_status_history WHERE camera_id = ?1"#,
        hid
    )
    .fetch_one(db)
    .await?;
    let items = sqlx::query_as!(
        StatusTransition,
        r#"SELECT id AS "id!: i64", occurred_at AS "occurred_at: OffsetDateTime",
                  from_state AS "from_state: CameraState", to_state AS "to_state: CameraState",
                  error_category AS "error_category: ErrorCategory"
             FROM camera_status_history WHERE camera_id = ?1
            ORDER BY id DESC LIMIT ?2 OFFSET ?3"#,
        hid,
        limit,
        offset
    )
    .fetch_all(db)
    .await?;
    Ok((items, total))
}

pub async fn count(db: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(r#"SELECT COUNT(*) AS "n!: i64" FROM cameras"#)
        .fetch_one(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage;

    fn new(name: &str, host: &str) -> CameraSettings {
        CameraSettings::from_new(NewCamera {
            name: name.into(),
            host: host.into(),
            port: None,
            scheme: Scheme::Https,
            tls_mode: TlsMode::Strict,
            tls_pinned_sha256: None,
            monitoring_enabled: true,
        })
        .unwrap()
    }

    async fn db() -> SqlitePool {
        storage::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_get_and_duplicate() {
        let db = db().await;
        let cam = create(&db, &new("Crane 1", "192.0.2.1")).await.unwrap();
        assert_eq!(cam.summary.port, 443);
        assert_eq!(cam.summary.status.state, CameraState::Unknown);
        assert!(cam.capabilities.is_empty());
        assert_eq!(get(&db, cam.summary.id).await.unwrap(), Some(cam));
        assert!(matches!(
            create(&db, &new("Other", "192.0.2.1")).await,
            Err(CameraError::Duplicate)
        ));
    }

    #[tokio::test]
    async fn pinned_requires_valid_fingerprint() {
        let base = NewCamera {
            name: "c".into(),
            host: "cam.local".into(),
            port: None,
            scheme: Scheme::Https,
            tls_mode: TlsMode::Pinned,
            tls_pinned_sha256: None,
            monitoring_enabled: true,
        };
        assert_eq!(
            CameraSettings::from_new(base.clone()),
            Err(ValidationError::PinMissing)
        );
        let ok = CameraSettings::from_new(NewCamera {
            tls_pinned_sha256: Some("ab".repeat(32)),
            ..base
        })
        .unwrap();
        assert!(ok.tls_pinned_sha256.unwrap().starts_with("AB:"));
    }

    #[tokio::test]
    async fn status_transitions_are_recorded_only_when_given() {
        let db = db().await;
        let id = create(&db, &new("c", "192.0.2.2"))
            .await
            .unwrap()
            .summary
            .id;
        let now = OffsetDateTime::now_utc();
        let online = CameraStatusView {
            state: CameraState::Online,
            last_success_at: Some(now),
            last_change_at: Some(now),
            last_checked_at: Some(now),
            response_time_ms: Some(42),
            ..Default::default()
        };
        write_status(
            &db,
            id,
            &online,
            Some((CameraState::Unknown, CameraState::Online)),
        )
        .await
        .unwrap();
        write_status(&db, id, &online, None).await.unwrap();
        let (h, total) = history(&db, id, 10, 0).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(h[0].to_state, CameraState::Online);
        let cam = get(&db, id).await.unwrap().unwrap();
        assert_eq!(cam.summary.status.response_time_ms, Some(42));
    }

    #[tokio::test]
    async fn changing_host_resets_identity_and_status() {
        let db = db().await;
        let id = create(&db, &new("c", "192.0.2.3"))
            .await
            .unwrap()
            .summary
            .id;
        let device = DeviceInfo {
            manufacturer: Some("AXIS".into()),
            model: Some("M1".into()),
            ..Default::default()
        };
        save_identity(&db, id, &device, None, None).await.unwrap();
        let (_, after) = update(
            &db,
            id,
            CameraPatch {
                name: Some("renamed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            after.summary.model.as_deref(),
            Some("M1"),
            "rename keeps identity"
        );

        let (_, after) = update(
            &db,
            id,
            CameraPatch {
                host: Some("192.0.2.99".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(after.summary.model, None, "new host clears identity");
        assert!(after.identity_updated_at.is_none());
    }

    #[tokio::test]
    async fn list_filters_sorts_and_searches() {
        let db = db().await;
        for (name, host) in [
            ("Bravo", "192.0.2.5"),
            ("alpha", "192.0.2.4"),
            ("Charlie", "cam.local"),
        ] {
            create(&db, &new(name, host)).await.unwrap();
        }
        let params = |search, state, sort, descending| ListParams {
            search,
            state,
            sort,
            descending,
            limit: 50,
            offset: 0,
        };
        let (cams, total) = list(&db, params(None, None, CameraSort::Name, false))
            .await
            .unwrap();
        assert_eq!(total, 3);
        let names: Vec<_> = cams.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["alpha", "Bravo", "Charlie"],
            "case-insensitive name sort"
        );

        let (cams, _) = list(&db, params(Some("cam.l"), None, CameraSort::Name, false))
            .await
            .unwrap();
        assert_eq!(cams.len(), 1);

        let (cams, total) = list(
            &db,
            params(None, Some(CameraState::Online), CameraSort::Name, false),
        )
        .await
        .unwrap();
        assert_eq!((cams.len(), total), (0, 0));
    }

    #[tokio::test]
    async fn delete_cascades_status_and_history() {
        let db = db().await;
        let id = create(&db, &new("c", "192.0.2.6"))
            .await
            .unwrap()
            .summary
            .id;
        delete(&db, id).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM camera_status")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 0);
        assert!(matches!(delete(&db, id).await, Err(CameraError::NotFound)));
    }
}
