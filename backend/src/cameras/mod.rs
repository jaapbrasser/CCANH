//! Camera registry: persistence of cameras, their identity and their status.

pub mod model;

use sqlx::SqlitePool;
use time::OffsetDateTime;
use uuid::fmt::Hyphenated;
use uuid::Uuid;

pub use model::*;

use crate::credentials::{self as creds, CredentialError, CredentialSetError, CredentialStore};
use crate::providers::axis::tls::TlsPolicy;
use crate::providers::{
    CameraCapabilities, DeviceInfo, ErrorCategory, ProviderError, ProviderTarget, StreamProfile,
};
use crate::settings::CameraDefaults;

#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("camera not found")]
    NotFound,
    #[error("a camera with this host and port already exists")]
    Duplicate,
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Credentials(#[from] CredentialSetError),
    #[error("the selected credential set does not exist")]
    UnknownCredentialSet,
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
    identity_error_category: Option<ErrorCategory>,
    identity_error_message: Option<String>,
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
                identity_error_category: r.identity_error_category,
                identity_error_message: r.identity_error_message,
            },
            groups: Vec::new(),
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

#[derive(Debug, Default)]
pub struct ListParams<'a> {
    pub search: Option<&'a str>,
    pub state: Option<CameraState>,
    pub sort: CameraSort,
    pub descending: bool,
    pub limit: i64,
    pub offset: i64,
    /// Cameras in this group or any of its subgroups.
    pub group: Option<Uuid>,
    /// Only this user's favorites.
    pub favorites_of: Option<Uuid>,
}

/// Fill in each summary's groups.
pub async fn attach_memberships(
    db: &SqlitePool,
    items: &mut [CameraSummary],
) -> Result<(), sqlx::Error> {
    let ids: Vec<Uuid> = items.iter().map(|c| c.id).collect();
    let mut m = crate::organization::memberships(db, &ids).await?;
    for c in items {
        if let Some(groups) = m.remove(&c.id) {
            c.groups = groups;
        }
    }
    Ok(())
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
    let group = p.group.map(Uuid::hyphenated);
    let favorites_of = p.favorites_of.map(Uuid::hyphenated);
    let sort = p.sort.as_str();
    let order = if p.descending { "desc" } else { "asc" };

    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64"
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE (?1 IS NULL OR c.name LIKE ?1 ESCAPE '\' OR c.host LIKE ?1 ESCAPE '\'
                   OR c.model LIKE ?1 ESCAPE '\')
              AND (?2 IS NULL OR s.state = ?2)
              AND (?3 IS NULL OR c.id IN (
                    SELECT cg.camera_id FROM camera_groups cg
                     WHERE cg.group_id IN (
                       WITH RECURSIVE sub(id) AS (
                         SELECT ?3 UNION SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id
                       ) SELECT id FROM sub)))
              AND (?4 IS NULL OR c.id IN (SELECT f.camera_id FROM favorites f WHERE f.user_id = ?4))"#,
        pattern,
        state,
        group,
        favorites_of
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
                  s.error_category AS "error_category: ErrorCategory", s.error_message,
                  s.identity_error_category AS "identity_error_category: ErrorCategory",
                  s.identity_error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE (?1 IS NULL OR c.name LIKE ?1 ESCAPE '\' OR c.host LIKE ?1 ESCAPE '\'
                   OR c.model LIKE ?1 ESCAPE '\')
              AND (?2 IS NULL OR s.state = ?2)
              AND (?7 IS NULL OR c.id IN (
                    SELECT cg.camera_id FROM camera_groups cg
                     WHERE cg.group_id IN (
                       WITH RECURSIVE sub(id) AS (
                         SELECT ?7 UNION SELECT g.id FROM groups g JOIN sub ON g.parent_id = sub.id
                       ) SELECT id FROM sub)))
              AND (?8 IS NULL OR c.id IN (SELECT f.camera_id FROM favorites f WHERE f.user_id = ?8))
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
        p.offset,
        group,
        favorites_of
    )
    .fetch_all(db)
    .await?;

    let mut items: Vec<CameraSummary> = rows.into_iter().map(CameraSummary::from).collect();
    attach_memberships(db, &mut items).await?;
    Ok((items, total))
}

/// Status counts of the monitored cameras (cameras with monitoring off are
/// left out), for the top bar (issue #32).
pub async fn status_summary(db: &SqlitePool) -> Result<StatusSummary, sqlx::Error> {
    let r = sqlx::query!(
        r#"SELECT COALESCE(SUM(s.state = 'online'), 0) AS "online!: i64",
                  COALESCE(SUM(s.state = 'offline'), 0) AS "offline!: i64",
                  COALESCE(SUM(s.state = 'unknown'), 0) AS "unknown!: i64"
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
            WHERE c.monitoring_enabled = 1"#
    )
    .fetch_one(db)
    .await?;
    let n = |v: i64| u32::try_from(v).unwrap_or(u32::MAX);
    Ok(StatusSummary {
        online: n(r.online),
        offline: n(r.offline),
        unknown: n(r.unknown),
    })
}

pub async fn get(db: &SqlitePool, id: Uuid) -> Result<Option<CameraDetail>, sqlx::Error> {
    let hid = id.hyphenated();
    let row = sqlx::query!(
        r#"SELECT c.id AS "id: Hyphenated", c.name, c.host, c.port, c.scheme AS "scheme: Scheme",
                  c.manufacturer, c.model, c.firmware_version,
                  c.monitoring_enabled AS "monitoring_enabled: bool",
                  c.tls_mode AS "tls_mode: TlsMode", c.tls_pinned_sha256,
                  c.name_auto AS "name_auto: bool",
                  c.credential_set_id AS "credential_set_id: Hyphenated", cs.name AS credential_set_name,
                  c.allow_basic_over_http AS "allow_basic_over_http: bool",
                  c.serial_number, c.mac_address, c.device_name, c.device_hostname,
                  c.capabilities, c.stream_profiles, c.network,
                  c.grid_profile, c.detail_profile,
                  c.identity_updated_at AS "identity_updated_at: OffsetDateTime",
                  c.created_at AS "created_at: OffsetDateTime",
                  c.updated_at AS "updated_at: OffsetDateTime",
                  s.state AS "state: CameraState",
                  s.last_success_at AS "last_success_at: OffsetDateTime",
                  s.last_failure_at AS "last_failure_at: OffsetDateTime",
                  s.last_change_at AS "last_change_at: OffsetDateTime",
                  s.last_checked_at AS "last_checked_at: OffsetDateTime",
                  s.response_time_ms, s.consecutive_failures,
                  s.error_category AS "error_category: ErrorCategory", s.error_message,
                  s.identity_error_category AS "identity_error_category: ErrorCategory",
                  s.identity_error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
             LEFT JOIN credential_sets cs ON cs.id = c.credential_set_id
            WHERE c.id = ?1"#,
        hid
    )
    .fetch_optional(db)
    .await?;

    let Some(mut detail) = row.map(|r| CameraDetail {
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
            identity_error_category: r.identity_error_category,
            identity_error_message: r.identity_error_message,
        }),
        tls_mode: (r.scheme == Scheme::Https).then_some(r.tls_mode),
        tls_pinned_sha256: r.tls_pinned_sha256,
        name_pending: r.name_auto,
        has_credentials: r.credential_set_id.is_some(),
        credential_set: r
            .credential_set_id
            .zip(r.credential_set_name)
            .map(|(id, name)| CredentialSetRef {
                id: id.into_uuid(),
                name,
            }),
        allow_basic_over_http: r.allow_basic_over_http,
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
        grid_profile: r.grid_profile,
        detail_profile: r.detail_profile,
        identity_updated_at: r.identity_updated_at,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }) else {
        return Ok(None);
    };
    attach_memberships(db, std::slice::from_mut(&mut detail.summary)).await?;
    Ok(Some(detail))
}

/// Set a camera's stream profile overrides. Returns `false` if the camera
/// doesn't exist.
pub async fn set_stream_overrides(
    db: &SqlitePool,
    id: Uuid,
    o: &StreamProfileOverrides,
) -> Result<bool, sqlx::Error> {
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let done = sqlx::query!(
        "UPDATE cameras SET grid_profile = ?1, detail_profile = ?2, updated_at = ?3 WHERE id = ?4",
        o.grid_profile,
        o.detail_profile,
        now,
        hid
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Validated, complete settings for inserting or updating a camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSettings {
    pub name: String,
    /// `name` is a placeholder to replace with the camera's own name (issue #14).
    pub name_auto: bool,
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    pub tls_mode: TlsMode,
    pub tls_pinned_sha256: Option<String>,
    pub monitoring_enabled: bool,
    pub allow_basic_over_http: bool,
}

/// What to do with a camera's credentials.
pub enum CredentialChange {
    Keep,
    Set(CredentialInput),
}

impl CameraSettings {
    /// Validate a new camera, splitting off its credential input. Fields the
    /// request leaves out come from the camera defaults setting (issue #18).
    pub fn from_new(
        n: NewCamera,
        defaults: &CameraDefaults,
    ) -> Result<(Self, Option<CredentialInput>), ValidationError> {
        let scheme = n.scheme.unwrap_or(defaults.scheme);
        let settings = Self::validated(Self {
            name: n.name.unwrap_or_default(),
            name_auto: false,
            host: n.host,
            port: n.port.unwrap_or_else(|| scheme.default_port()),
            scheme,
            tls_mode: n.tls_mode.unwrap_or(defaults.tls_mode),
            tls_pinned_sha256: n.tls_pinned_sha256,
            monitoring_enabled: n.monitoring_enabled,
            allow_basic_over_http: n
                .allow_basic_over_http
                .unwrap_or(defaults.allow_basic_over_http),
        })?;
        Ok((settings, n.credentials))
    }

    /// The TLS policy providers should use.
    pub fn tls_policy(&self) -> TlsPolicy {
        tls_policy(self.tls_mode, self.tls_pinned_sha256.as_deref())
    }

    /// Apply a patch to the current settings. A camera switched from HTTP to
    /// HTTPS without a certificate mode gets the default one.
    pub fn patched(
        current: &CameraDetail,
        p: CameraPatch,
        defaults: &CameraDefaults,
    ) -> Result<(Self, CredentialChange), ValidationError> {
        let s = &current.summary;
        let current_tls = current.tls_mode.unwrap_or(defaults.tls_mode);
        let tls_mode = p.tls_mode.unwrap_or(current_tls);
        let pin = match (p.tls_pinned_sha256, tls_mode) {
            (Some(pin), _) => Some(pin),
            (None, TlsMode::Pinned) => current.tls_pinned_sha256.clone(),
            (None, _) => None,
        };
        // Keep a placeholder name a placeholder unless the admin names it.
        let (name, name_auto) = match p.name {
            Some(name) => (name, false),
            None if current.name_pending => (String::new(), false),
            None => (s.name.clone(), false),
        };
        let settings = Self::validated(Self {
            name,
            name_auto,
            host: p.host.unwrap_or_else(|| s.host.clone()),
            port: p.port.unwrap_or(s.port),
            scheme: p.scheme.unwrap_or(s.scheme),
            tls_mode,
            tls_pinned_sha256: pin,
            monitoring_enabled: p.monitoring_enabled.unwrap_or(s.monitoring_enabled),
            allow_basic_over_http: p
                .allow_basic_over_http
                .unwrap_or(current.allow_basic_over_http),
        })?;
        let change = p
            .credentials
            .map_or(CredentialChange::Keep, CredentialChange::Set);
        Ok((settings, change))
    }

    fn validated(mut s: Self) -> Result<Self, ValidationError> {
        s.host = validate_host(&s.host)?;
        s.port = validate_port(s.port)?;
        if s.name.trim().is_empty() {
            // No name: show the address until the camera reports its own.
            s.name = address_label(&s.host, s.port, s.scheme);
            s.name_auto = true;
        } else {
            s.name = validate_name(&s.name)?;
        }
        if s.scheme == Scheme::Http {
            // No certificate check over HTTP (issue #17). The column is NOT NULL,
            // so store the neutral value; the API reports `null`.
            s.tls_mode = TlsMode::Strict;
            s.tls_pinned_sha256 = None;
        }
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

pub fn tls_policy(mode: TlsMode, pin: Option<&str>) -> TlsPolicy {
    match (mode, pin) {
        (TlsMode::Pinned, Some(fp)) => TlsPolicy::Pinned(fp.to_owned()),
        // Validation guarantees a pin in pinned mode; fail closed if not.
        (TlsMode::Pinned, None) => TlsPolicy::Pinned(String::new()),
        (TlsMode::InsecureSkipVerify, _) => TlsPolicy::InsecureSkipVerify,
        (TlsMode::Strict, _) => TlsPolicy::Strict,
    }
}

/// Resolve a credential input to a credential set id within a transaction,
/// creating a private set for inline credentials.
async fn resolve_credentials(
    tx: &mut sqlx::SqliteConnection,
    store: &dyn CredentialStore,
    input: CredentialInput,
) -> Result<Option<Uuid>, CameraError> {
    match input {
        CredentialInput::None => Ok(None),
        CredentialInput::Set { id } => {
            let hid = id.hyphenated();
            let shared = sqlx::query_scalar!(
                r#"SELECT shared AS "shared: bool" FROM credential_sets WHERE id = ?1"#,
                hid
            )
            .fetch_optional(&mut *tx)
            .await?;
            match shared {
                Some(true) => Ok(Some(id)),
                _ => Err(CameraError::UnknownCredentialSet),
            }
        }
        CredentialInput::Inline { username, password } => {
            let c = creds::Credentials { username, password };
            Ok(Some(creds::create_private(tx, store, &c).await?))
        }
    }
}

pub async fn create(
    db: &SqlitePool,
    store: &dyn CredentialStore,
    s: &CameraSettings,
    credentials: Option<CredentialInput>,
) -> Result<CameraDetail, CameraError> {
    let id = Uuid::now_v7();
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let port = i64::from(s.port);
    let mut tx = db.begin().await?;
    let set_id = match credentials {
        Some(input) => resolve_credentials(&mut tx, store, input).await?,
        None => None,
    };
    let set_hid = set_id.map(|u| u.hyphenated());
    let inserted = sqlx::query!(
        "INSERT INTO cameras (id, name, host, port, scheme, tls_mode, tls_pinned_sha256,
                              monitoring_enabled, credential_set_id, allow_basic_over_http,
                              created_at, updated_at, name_auto)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
        hid,
        s.name,
        s.host,
        port,
        s.scheme,
        s.tls_mode,
        s.tls_pinned_sha256,
        s.monitoring_enabled,
        set_hid,
        s.allow_basic_over_http,
        now,
        s.name_auto
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
    store: &dyn CredentialStore,
    id: Uuid,
    patch: CameraPatch,
    defaults: &CameraDefaults,
) -> Result<(CameraDetail, CameraDetail), CameraError> {
    let before = get(db, id).await?.ok_or(CameraError::NotFound)?;
    let (s, change) = CameraSettings::patched(&before, patch, defaults)?;
    let reset = s.targets_differ(&before);
    let hid = id.hyphenated();
    let now = OffsetDateTime::now_utc();
    let port = i64::from(s.port);

    let mut tx = db.begin().await?;
    let old_set: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT credential_set_id AS "id: Hyphenated" FROM cameras WHERE id = ?1"#,
        hid
    )
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .map(Hyphenated::into_uuid);
    let new_set = match change {
        CredentialChange::Keep => old_set,
        CredentialChange::Set(input) => resolve_credentials(&mut tx, store, input).await?,
    };
    let new_set_hid = new_set.map(|u| u.hyphenated());
    let updated = sqlx::query!(
        "UPDATE cameras
            SET name = ?2, host = ?3, port = ?4, scheme = ?5, tls_mode = ?6,
                tls_pinned_sha256 = ?7, monitoring_enabled = ?8, credential_set_id = ?9,
                allow_basic_over_http = ?10, updated_at = ?11, name_auto = ?12
          WHERE id = ?1",
        hid,
        s.name,
        s.host,
        port,
        s.scheme,
        s.tls_mode,
        s.tls_pinned_sha256,
        s.monitoring_enabled,
        new_set_hid,
        s.allow_basic_over_http,
        now,
        s.name_auto
    )
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(r) if r.rows_affected() == 0 => return Err(CameraError::NotFound),
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(CameraError::Duplicate),
        Err(e) => return Err(e.into()),
    }
    if let Some(old) = old_set.filter(|o| Some(*o) != new_set) {
        creds::delete_private_if_unused(&mut tx, old).await?;
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
                    error_category = NULL, error_message = NULL,
                    identity_error_category = NULL, identity_error_message = NULL
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
    let mut tx = db.begin().await?;
    let set: Option<Uuid> = sqlx::query_scalar!(
        r#"SELECT credential_set_id AS "id: Hyphenated" FROM cameras WHERE id = ?1"#,
        hid
    )
    .fetch_optional(&mut *tx)
    .await?
    .flatten()
    .map(Hyphenated::into_uuid);
    let result = sqlx::query!("DELETE FROM cameras WHERE id = ?1", hid)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(CameraError::NotFound);
    }
    if let Some(set) = set {
        creds::delete_private_if_unused(&mut tx, set).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// What a provider (and the status monitor) needs for one camera. Holds the
/// credential *ciphertext*; decrypt only when building a provider.
#[derive(Clone, PartialEq, Eq)]
pub struct MonitorTarget {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    pub tls_mode: TlsMode,
    pub tls_pinned_sha256: Option<String>,
    pub allow_basic_over_http: bool,
    pub credential_secret: Option<Vec<u8>>,
    pub updated_at: OffsetDateTime,
    /// Also changes when a shared credential set is edited.
    pub credentials_updated_at: Option<OffsetDateTime>,
    pub identity_updated_at: Option<OffsetDateTime>,
    pub status: CameraStatusView,
}

impl std::fmt::Debug for MonitorTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorTarget")
            .field("id", &self.id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("has_credentials", &self.credential_secret.is_some())
            .finish_non_exhaustive()
    }
}

impl MonitorTarget {
    /// Decrypt credentials and build the provider target.
    pub fn provider_target(
        &self,
        store: &dyn CredentialStore,
    ) -> Result<ProviderTarget, CredentialError> {
        let credentials = self
            .credential_secret
            .as_deref()
            .map(|s| store.open(s))
            .transpose()?;
        Ok(ProviderTarget {
            host: self.host.clone(),
            port: self.port,
            https: self.scheme == Scheme::Https,
            tls: tls_policy(self.tls_mode, self.tls_pinned_sha256.as_deref()),
            credentials,
            allow_basic_over_http: self.allow_basic_over_http,
        })
    }

    /// Key that changes whenever the provider must be rebuilt.
    pub fn config_version(&self) -> (OffsetDateTime, Option<OffsetDateTime>) {
        (self.updated_at, self.credentials_updated_at)
    }
}

/// Monitored cameras (or one camera by id, monitored or not).
pub async fn monitor_targets(db: &SqlitePool) -> Result<Vec<MonitorTarget>, sqlx::Error> {
    targets(db, None).await
}

pub async fn target(db: &SqlitePool, id: Uuid) -> Result<Option<MonitorTarget>, sqlx::Error> {
    Ok(targets(db, Some(id)).await?.into_iter().next())
}

async fn targets(db: &SqlitePool, only: Option<Uuid>) -> Result<Vec<MonitorTarget>, sqlx::Error> {
    let only = only.map(|u| u.hyphenated());
    let rows = sqlx::query!(
        r#"SELECT c.id AS "id: Hyphenated", c.host, c.port, c.scheme AS "scheme: Scheme",
                  c.tls_mode AS "tls_mode: TlsMode", c.tls_pinned_sha256,
                  c.allow_basic_over_http AS "allow_basic_over_http: bool",
                  cs.secret AS "credential_secret?: Vec<u8>",
                  cs.updated_at AS "credentials_updated_at?: OffsetDateTime",
                  c.updated_at AS "updated_at: OffsetDateTime",
                  c.identity_updated_at AS "identity_updated_at: OffsetDateTime",
                  s.state AS "state: CameraState",
                  s.last_success_at AS "last_success_at: OffsetDateTime",
                  s.last_failure_at AS "last_failure_at: OffsetDateTime",
                  s.last_change_at AS "last_change_at: OffsetDateTime",
                  s.last_checked_at AS "last_checked_at: OffsetDateTime",
                  s.response_time_ms, s.consecutive_failures,
                  s.error_category AS "error_category: ErrorCategory", s.error_message,
                  s.identity_error_category AS "identity_error_category: ErrorCategory",
                  s.identity_error_message
             FROM cameras c JOIN camera_status s ON s.camera_id = c.id
             LEFT JOIN credential_sets cs ON cs.id = c.credential_set_id
            WHERE (?1 IS NULL AND c.monitoring_enabled = 1) OR c.id = ?1"#,
        only
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
            tls_mode: r.tls_mode,
            tls_pinned_sha256: r.tls_pinned_sha256,
            allow_basic_over_http: r.allow_basic_over_http,
            credential_secret: r.credential_secret,
            updated_at: r.updated_at,
            credentials_updated_at: r.credentials_updated_at,
            identity_updated_at: r.identity_updated_at,
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
                identity_error_category: r.identity_error_category,
                identity_error_message: r.identity_error_message,
            },
        })
        .collect())
}

/// Record the outcome of an identity/credential check (`None` = success).
pub async fn record_identity_result(
    db: &SqlitePool,
    id: Uuid,
    error: Option<&ProviderError>,
) -> Result<(), sqlx::Error> {
    let hid = id.hyphenated();
    let category = error.map(ProviderError::category);
    let message = error.map(|e| e.to_string().chars().take(300).collect::<String>());
    sqlx::query!(
        "UPDATE camera_status SET identity_error_category = ?2, identity_error_message = ?3
          WHERE camera_id = ?1",
        hid,
        category,
        message
    )
    .execute(db)
    .await?;
    Ok(())
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
/// `None` for capabilities/profiles keeps what's stored. A camera nobody named
/// gets its own name (issue #14); returns that name when it does.
pub async fn save_identity(
    db: &SqlitePool,
    id: Uuid,
    device: &DeviceInfo,
    capabilities: Option<&CameraCapabilities>,
    profiles: Option<&[StreamProfile]>,
) -> Result<Option<String>, sqlx::Error> {
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
    let Some(name) = auto_name(device) else {
        return Ok(None);
    };
    let renamed = sqlx::query!(
        "UPDATE cameras SET name = ?2, name_auto = 0, updated_at = ?3
          WHERE id = ?1 AND name_auto = 1",
        hid,
        name,
        now
    )
    .execute(db)
    .await?;
    Ok((renamed.rows_affected() == 1).then_some(name))
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

    fn defaults() -> CameraDefaults {
        CameraDefaults::from(&crate::config::CameraDefaultsConfig::default())
    }

    fn new(name: &str, host: &str) -> CameraSettings {
        CameraSettings::from_new(
            NewCamera {
                name: Some(name.into()),
                host: host.into(),
                port: None,
                scheme: Some(Scheme::Https),
                tls_mode: Some(TlsMode::Strict),
                tls_pinned_sha256: None,
                monitoring_enabled: true,
                credentials: None,
                allow_basic_over_http: Some(false),
                group_ids: vec![],
            },
            &defaults(),
        )
        .unwrap()
        .0
    }

    fn bare(host: &str) -> NewCamera {
        NewCamera {
            name: None,
            host: host.into(),
            port: None,
            scheme: None,
            tls_mode: None,
            tls_pinned_sha256: None,
            monitoring_enabled: true,
            credentials: None,
            allow_basic_over_http: None,
            group_ids: vec![],
        }
    }

    #[test]
    fn omitted_fields_come_from_the_camera_defaults() {
        let (s, _) = CameraSettings::from_new(bare("192.0.2.9"), &defaults()).unwrap();
        assert_eq!(s.scheme, Scheme::Http);
        assert_eq!(s.port, 80);
        assert!(s.allow_basic_over_http);
        // Certificate settings don't apply over HTTP.
        assert_eq!(s.tls_mode, TlsMode::Strict);
        assert_eq!(s.tls_pinned_sha256, None);

        let strict = CameraDefaults {
            scheme: Scheme::Https,
            allow_basic_over_http: false,
            tls_mode: TlsMode::Strict,
        };
        let (s, _) = CameraSettings::from_new(bare("192.0.2.9"), &strict).unwrap();
        assert_eq!(
            (s.scheme, s.port, s.allow_basic_over_http),
            (Scheme::Https, 443, false)
        );
        // Explicit values win over the defaults.
        let mut n = bare("192.0.2.9");
        n.scheme = Some(Scheme::Https);
        n.tls_mode = Some(TlsMode::Strict);
        let (s, _) = CameraSettings::from_new(n, &defaults()).unwrap();
        assert_eq!(s.tls_mode, TlsMode::Strict);
    }

    #[tokio::test]
    async fn unnamed_cameras_show_their_address_until_identified() {
        let db = db().await;
        let mut n = bare("192.0.2.10");
        n.port = Some(8080);
        let (s, _) = CameraSettings::from_new(n, &defaults()).unwrap();
        assert!(s.name_auto);
        let cam = create(&db, &*store(), &s, None).await.unwrap();
        assert_eq!(cam.summary.name, "192.0.2.10:8080");
        assert!(cam.name_pending);
        assert_eq!(
            cam.tls_mode, None,
            "HTTP cameras report no certificate mode"
        );

        let id = cam.summary.id;
        let device = DeviceInfo {
            device_name: Some("Hook cam".into()),
            ..Default::default()
        };
        let named = save_identity(&db, id, &device, None, None).await.unwrap();
        assert_eq!(named.as_deref(), Some("Hook cam"));
        let cam = get(&db, id).await.unwrap().unwrap();
        assert_eq!(cam.summary.name, "Hook cam");
        assert!(!cam.name_pending);

        // Set once: a later change on the camera doesn't rename it.
        let renamed = DeviceInfo {
            device_name: Some("Other".into()),
            ..Default::default()
        };
        assert_eq!(
            save_identity(&db, id, &renamed, None, None).await.unwrap(),
            None
        );
        assert_eq!(
            get(&db, id).await.unwrap().unwrap().summary.name,
            "Hook cam"
        );
    }

    #[tokio::test]
    async fn admin_names_are_never_replaced_and_blank_means_auto() {
        let db = db().await;
        let cam = create(&db, &*store(), &new("Crane 1", "192.0.2.11"), None)
            .await
            .unwrap();
        let id = cam.summary.id;
        let device = DeviceInfo {
            device_name: Some("Hook cam".into()),
            ..Default::default()
        };
        assert_eq!(
            save_identity(&db, id, &device, None, None).await.unwrap(),
            None
        );
        assert_eq!(get(&db, id).await.unwrap().unwrap().summary.name, "Crane 1");

        // Clearing the name hands it back to the camera.
        let patch = CameraPatch {
            name: Some(String::new()),
            ..Default::default()
        };
        let (_, after) = update(&db, &*store(), id, patch, &defaults())
            .await
            .unwrap();
        assert_eq!(after.summary.name, "192.0.2.11");
        assert!(after.name_pending);
    }

    #[tokio::test]
    async fn switching_to_http_clears_certificate_settings() {
        let db = db().await;
        let mut n = bare("192.0.2.12");
        n.scheme = Some(Scheme::Https);
        n.tls_mode = Some(TlsMode::Pinned);
        n.tls_pinned_sha256 = Some("ab".repeat(32));
        let (s, _) = CameraSettings::from_new(n, &defaults()).unwrap();
        let cam = create(&db, &*store(), &s, None).await.unwrap();
        assert_eq!(cam.tls_mode, Some(TlsMode::Pinned));

        let to_http = CameraPatch {
            scheme: Some(Scheme::Http),
            port: Some(80),
            ..Default::default()
        };
        let (_, after) = update(&db, &*store(), cam.summary.id, to_http, &defaults())
            .await
            .unwrap();
        assert_eq!(after.tls_mode, None);
        assert_eq!(after.tls_pinned_sha256, None);

        // Back to HTTPS without a mode: the default certificate check applies.
        let to_https = CameraPatch {
            scheme: Some(Scheme::Https),
            port: Some(443),
            ..Default::default()
        };
        let (_, after) = update(&db, &*store(), cam.summary.id, to_https, &defaults())
            .await
            .unwrap();
        assert_eq!(after.tls_mode, Some(TlsMode::InsecureSkipVerify));
    }

    fn store() -> Box<dyn CredentialStore> {
        Box::new(crate::credentials::AesGcmStore::ephemeral())
    }

    async fn db() -> SqlitePool {
        storage::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn create_get_and_duplicate() {
        let db = db().await;
        let cam = create(&db, &*store(), &new("Crane 1", "192.0.2.1"), None)
            .await
            .unwrap();
        assert_eq!(cam.summary.port, 443);
        assert_eq!(cam.summary.status.state, CameraState::Unknown);
        assert!(cam.capabilities.is_empty());
        assert_eq!(get(&db, cam.summary.id).await.unwrap(), Some(cam));
        assert!(matches!(
            create(&db, &*store(), &new("Other", "192.0.2.1"), None).await,
            Err(CameraError::Duplicate)
        ));
    }

    #[tokio::test]
    async fn pinned_requires_valid_fingerprint() {
        let base = |pin: Option<String>| NewCamera {
            name: Some("c".into()),
            host: "cam.local".into(),
            port: None,
            scheme: Some(Scheme::Https),
            tls_mode: Some(TlsMode::Pinned),
            tls_pinned_sha256: pin,
            monitoring_enabled: true,
            credentials: None,
            allow_basic_over_http: Some(false),
            group_ids: vec![],
        };
        assert_eq!(
            CameraSettings::from_new(base(None), &defaults()).err(),
            Some(ValidationError::PinMissing)
        );
        let (ok, _) = CameraSettings::from_new(base(Some("ab".repeat(32))), &defaults()).unwrap();
        assert!(ok.tls_pinned_sha256.clone().unwrap().starts_with("AB:"));
        assert!(matches!(ok.tls_policy(), TlsPolicy::Pinned(fp) if fp.starts_with("AB:")));
    }

    #[tokio::test]
    async fn status_transitions_are_recorded_only_when_given() {
        let db = db().await;
        let id = create(&db, &*store(), &new("c", "192.0.2.2"), None)
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
        let id = create(&db, &*store(), &new("c", "192.0.2.3"), None)
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
            &*store(),
            id,
            CameraPatch {
                name: Some("renamed".into()),
                ..Default::default()
            },
            &defaults(),
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
            &*store(),
            id,
            CameraPatch {
                host: Some("192.0.2.99".into()),
                ..Default::default()
            },
            &defaults(),
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
            create(&db, &*store(), &new(name, host), None)
                .await
                .unwrap();
        }
        let params = |search, state, sort, descending| ListParams {
            search,
            state,
            sort,
            descending,
            limit: 50,
            offset: 0,
            ..Default::default()
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
    async fn credentials_lifecycle() {
        use crate::secret::{ExposeSecret, SecretString};
        let db = db().await;
        let store = store();
        let inline = |user: &str| CredentialInput::Inline {
            username: user.into(),
            password: SecretString::from("cam-pass-123".to_owned()),
        };
        let cam = create(&db, &*store, &new("c", "192.0.2.7"), Some(inline("root")))
            .await
            .unwrap();
        assert!(cam.has_credentials);
        assert!(
            cam.credential_set.is_none(),
            "inline credentials are private"
        );
        let id = cam.summary.id;

        // The target decrypts the credentials for the provider.
        let t = target(&db, id).await.unwrap().unwrap();
        let pt = t.provider_target(&*store).unwrap();
        let c = pt.credentials.unwrap();
        assert_eq!(c.username, "root");
        assert_eq!(c.password.expose_secret(), "cam-pass-123");
        assert!(!format!("{t:?}").contains("cam-pass"));

        // Switching to a shared set removes the private one.
        let shared = crate::credentials::create_shared(
            &db,
            &*store,
            "Site A",
            &crate::credentials::Credentials {
                username: "svc".into(),
                password: SecretString::from("shared-pass-1".to_owned()),
            },
        )
        .await
        .unwrap();
        let (_, after) = update(
            &db,
            &*store,
            id,
            CameraPatch {
                credentials: Some(CredentialInput::Set { id: shared.id }),
                ..Default::default()
            },
            &defaults(),
        )
        .await
        .unwrap();
        assert_eq!(after.credential_set.as_ref().unwrap().name, "Site A");
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credential_sets WHERE shared = 0")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 0, "orphaned private set was removed");

        // A shared set in use can't be deleted.
        assert!(matches!(
            crate::credentials::delete_shared(&db, shared.id).await,
            Err(CredentialSetError::InUse(1))
        ));

        // Unknown sets are rejected.
        let bad = update(
            &db,
            &*store,
            id,
            CameraPatch {
                credentials: Some(CredentialInput::Set { id: Uuid::now_v7() }),
                ..Default::default()
            },
            &defaults(),
        )
        .await;
        assert!(matches!(bad, Err(CameraError::UnknownCredentialSet)));

        // Removing credentials.
        let (_, after) = update(
            &db,
            &*store,
            id,
            CameraPatch {
                credentials: Some(CredentialInput::None),
                ..Default::default()
            },
            &defaults(),
        )
        .await
        .unwrap();
        assert!(!after.has_credentials);
    }

    #[tokio::test]
    async fn deleting_camera_removes_its_private_credentials() {
        use crate::secret::SecretString;
        let db = db().await;
        let store = store();
        let cam = create(
            &db,
            &*store,
            &new("c", "192.0.2.8"),
            Some(CredentialInput::Inline {
                username: "root".into(),
                password: SecretString::from("x-pass-12345".to_owned()),
            }),
        )
        .await
        .unwrap();
        delete(&db, cam.summary.id).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM credential_sets")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn delete_cascades_status_and_history() {
        let db = db().await;
        let id = create(&db, &*store(), &new("c", "192.0.2.6"), None)
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
