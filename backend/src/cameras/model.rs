//! Camera API models and input validation.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::organization::GroupRef;
use crate::providers::{Capability, ErrorCategory, NetworkInfo, StreamProfile};
use crate::secret::SecretString;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type, Default,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum Scheme {
    Http,
    #[default]
    Https,
}

impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

/// Per-camera certificate validation (SPEC §4.5).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type, Default,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TlsMode {
    #[default]
    Strict,
    /// Trust exactly the certificate with `tls_pinned_sha256`.
    Pinned,
    /// No certificate validation. Must be chosen explicitly; shown with a warning.
    InsecureSkipVerify,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type, Default,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum CameraState {
    #[default]
    Unknown,
    Online,
    Offline,
}

impl CameraState {
    pub fn as_str(self) -> &'static str {
        match self {
            CameraState::Unknown => "unknown",
            CameraState::Online => "online",
            CameraState::Offline => "offline",
        }
    }
}

/// Current monitoring status of a camera.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, Default)]
pub struct CameraStatusView {
    pub state: CameraState,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_success_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_failure_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_change_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_checked_at: Option<OffsetDateTime>,
    pub response_time_ms: Option<u32>,
    pub consecutive_failures: u32,
    pub error_category: Option<ErrorCategory>,
    /// Short, non-sensitive description of the last error.
    pub error_message: Option<String>,
    /// Result of the last identity/credential check, separate from
    /// reachability: a camera can be online but reject its credentials.
    pub identity_error_category: Option<ErrorCategory>,
    pub identity_error_message: Option<String>,
}

/// A camera as shown in lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CameraSummary {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub monitoring_enabled: bool,
    pub status: CameraStatusView,
    /// Groups the camera is directly in, by name.
    pub groups: Vec<GroupRef>,
}

/// A reference to a shared credential set (never contains secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CredentialSetRef {
    pub id: Uuid,
    pub name: String,
}

/// Everything known about a camera (SPEC §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CameraDetail {
    #[serde(flatten)]
    pub summary: CameraSummary,
    /// Certificate check; `null` for HTTP cameras, where it doesn't apply.
    pub tls_mode: Option<TlsMode>,
    pub tls_pinned_sha256: Option<String>,
    /// The name is a placeholder (the address) that will be replaced by the
    /// camera's own name on the next successful identity check.
    pub name_pending: bool,
    /// Whether credentials are configured. Credentials themselves are write-only.
    pub has_credentials: bool,
    /// The shared credential set in use, if any (`null` for per-camera credentials).
    pub credential_set: Option<CredentialSetRef>,
    pub allow_basic_over_http: bool,
    pub serial_number: Option<String>,
    pub mac_address: Option<String>,
    pub device_name: Option<String>,
    pub device_hostname: Option<String>,
    pub network: Option<NetworkInfo>,
    /// Supported and unsupported capabilities. Empty until the first successful probe.
    pub capabilities: Vec<Capability>,
    pub stream_profiles: Vec<StreamProfile>,
    /// Profile id for grid tiles on this camera; `null` uses the global default.
    pub grid_profile: Option<String>,
    /// Profile id for the detail view on this camera; `null` uses the global default.
    pub detail_profile: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub identity_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

/// Per-camera stream profile choices (SPEC §7.4). Each is a profile id from
/// the camera's `stream_profiles` (`""` is the camera's default stream), or
/// `null` for the global default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StreamProfileOverrides {
    pub grid_profile: Option<String>,
    pub detail_profile: Option<String>,
}

/// A recorded status transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct StatusTransition {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub occurred_at: OffsetDateTime,
    pub from_state: CameraState,
    pub to_state: CameraState,
    pub error_category: Option<ErrorCategory>,
}

/// How a camera authenticates. Credentials are write-only.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialInput {
    /// No credentials (anonymous calls only).
    None,
    /// Use a shared credential set.
    Set { id: Uuid },
    /// Credentials for this camera only.
    Inline {
        username: String,
        #[schema(value_type = String, format = Password)]
        password: SecretString,
    },
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewCamera {
    /// Optional: leave it out (or blank) and the camera's own name is filled
    /// in on the first successful connection; until then the address is shown.
    #[serde(default)]
    pub name: Option<String>,
    /// IP address or hostname, without scheme, port or path.
    pub host: String,
    /// Defaults to 443 for HTTPS, 80 for HTTP.
    pub port: Option<u16>,
    /// Omit to use the camera defaults setting.
    #[serde(default)]
    pub scheme: Option<Scheme>,
    /// HTTPS only; omit to use the camera defaults setting. Ignored for HTTP.
    #[serde(default)]
    pub tls_mode: Option<TlsMode>,
    /// SHA-256 certificate fingerprint (hex, colons optional). Required for `pinned`.
    pub tls_pinned_sha256: Option<String>,
    #[serde(default = "default_true")]
    pub monitoring_enabled: bool,
    #[serde(default)]
    pub credentials: Option<CredentialInput>,
    /// Allow Basic authentication over plain HTTP, which sends the password
    /// unencrypted. Omit to use the camera defaults setting (SPEC §4.5, ADR 0011).
    #[serde(default)]
    pub allow_basic_over_http: Option<bool>,
    /// Groups to put the camera in.
    #[serde(default)]
    pub group_ids: Vec<Uuid>,
}

fn default_true() -> bool {
    true
}

impl CredentialInput {
    /// A copy for applying the same credentials to several cameras.
    pub fn duplicate(&self) -> Self {
        match self {
            CredentialInput::None => CredentialInput::None,
            CredentialInput::Set { id } => CredentialInput::Set { id: *id },
            CredentialInput::Inline { username, password } => CredentialInput::Inline {
                username: username.clone(),
                password: password.clone(),
            },
        }
    }

    /// For audit entries: the kind of credentials, never the secret.
    pub fn describe(&self) -> serde_json::Value {
        match self {
            CredentialInput::None => serde_json::json!({ "type": "none" }),
            CredentialInput::Set { id } => serde_json::json!({ "type": "set", "id": id }),
            // Usernames are write-only too (SPEC §4.6).
            CredentialInput::Inline { .. } => serde_json::json!({ "type": "inline" }),
        }
    }
}

/// Partial update. Omitted fields are unchanged. Send `tls_pinned_sha256: null`
/// together with a non-pinned `tls_mode` to clear a pin.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CameraPatch {
    /// A blank name means "fill it in from the camera" (see `NewCamera::name`).
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub scheme: Option<Scheme>,
    pub tls_mode: Option<TlsMode>,
    pub tls_pinned_sha256: Option<String>,
    pub monitoring_enabled: Option<bool>,
    /// Omit to keep the current credentials.
    pub credentials: Option<CredentialInput>,
    pub allow_basic_over_http: Option<bool>,
    /// Replace the camera's groups.
    pub group_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("name must be 1–100 characters")]
    Name,
    #[error("host must be an IP address or hostname, without scheme, port, path or credentials")]
    Host,
    #[error("port must be between 1 and 65535")]
    Port,
    #[error("tls_pinned_sha256 must be a 64-character hex SHA-256 fingerprint")]
    Fingerprint,
    #[error("tls_mode \"pinned\" requires tls_pinned_sha256")]
    PinMissing,
}

pub const MAX_NAME_LEN: usize = 100;

/// The name for a camera nobody named: its device name, else model and serial
/// number, else its hostname (issue #14). `None` when it reported none of them.
pub fn auto_name(device: &crate::providers::DeviceInfo) -> Option<String> {
    fn clean(v: &Option<String>) -> Option<&str> {
        v.as_deref().map(str::trim).filter(|v| !v.is_empty())
    }
    let name = clean(&device.device_name)
        .map(str::to_owned)
        .or_else(
            || match (clean(&device.model), clean(&device.serial_number)) {
                (Some(m), Some(s)) => Some(format!("{m} {s}")),
                (Some(m), None) => Some(m.to_owned()),
                (None, Some(s)) => Some(s.to_owned()),
                (None, None) => None,
            },
        )
        .or_else(|| clean(&device.hostname).map(str::to_owned))?;
    Some(name.chars().take(MAX_NAME_LEN).collect())
}

/// What lists show for a camera that has no name yet.
pub fn address_label(host: &str, port: u16, scheme: Scheme) -> String {
    if port == scheme.default_port() {
        host.to_owned()
    } else if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub fn validate_name(name: &str) -> Result<String, ValidationError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return Err(ValidationError::Name);
    }
    Ok(name.to_owned())
}

/// Accept IPv4, IPv6 (no brackets) or an RFC 1123 hostname. Rejects anything that
/// looks like a URL, so credentials can never sneak in via `user:pass@host`.
pub fn validate_host(host: &str) -> Result<String, ValidationError> {
    let host = host.trim();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_owned());
    }
    let valid_label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    if trimmed.is_empty() || trimmed.len() > 253 || !trimmed.split('.').all(valid_label) {
        return Err(ValidationError::Host);
    }
    Ok(trimmed.to_ascii_lowercase())
}

pub fn validate_port(port: u16) -> Result<u16, ValidationError> {
    if port == 0 {
        return Err(ValidationError::Port);
    }
    Ok(port)
}

/// Normalize to uppercase colon-separated hex, matching `tls::fingerprint`.
pub fn normalize_fingerprint(input: &str) -> Result<String, ValidationError> {
    let hex: String = input
        .chars()
        .filter(|c| *c != ':' && !c.is_whitespace())
        .collect();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ValidationError::Fingerprint);
    }
    let upper = hex.to_ascii_uppercase();
    Ok(upper
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(validate_host(" 192.0.2.1 ").unwrap(), "192.0.2.1");
        assert_eq!(validate_host("2001:db8::1").unwrap(), "2001:db8::1");
        assert_eq!(
            validate_host("Cam-01.Site-A.local.").unwrap(),
            "cam-01.site-a.local"
        );
        for bad in [
            "",
            "http://192.0.2.1",
            "192.0.2.1:80",
            "admin:secret@cam.local",
            "cam.local/path",
            "-cam.local",
            "cam_01.local",
            "a..b",
            "[2001:db8::1]",
        ] {
            assert_eq!(validate_host(bad), Err(ValidationError::Host), "{bad}");
        }
    }

    #[test]
    fn auto_names_prefer_the_device_name() {
        use crate::providers::DeviceInfo;
        let mut d = DeviceInfo {
            device_name: Some(" Hook cam ".into()),
            model: Some("M3088-V".into()),
            serial_number: Some("B8A44F000001".into()),
            hostname: Some("axis-b8a44f000001".into()),
            ..Default::default()
        };
        assert_eq!(auto_name(&d).as_deref(), Some("Hook cam"));
        d.device_name = Some("  ".into());
        assert_eq!(auto_name(&d).as_deref(), Some("M3088-V B8A44F000001"));
        d.model = None;
        d.serial_number = None;
        assert_eq!(auto_name(&d).as_deref(), Some("axis-b8a44f000001"));
        assert_eq!(auto_name(&DeviceInfo::default()), None);
    }

    #[test]
    fn address_labels() {
        assert_eq!(address_label("192.0.2.1", 80, Scheme::Http), "192.0.2.1");
        assert_eq!(
            address_label("192.0.2.1", 8080, Scheme::Http),
            "192.0.2.1:8080"
        );
        assert_eq!(
            address_label("2001:db8::1", 8443, Scheme::Https),
            "[2001:db8::1]:8443"
        );
    }

    #[test]
    fn names() {
        assert_eq!(validate_name("  Crane 1 ").unwrap(), "Crane 1");
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(101)).is_err());
    }

    #[test]
    fn fingerprints() {
        let raw = "ab".repeat(32);
        let norm = normalize_fingerprint(&raw).unwrap();
        assert_eq!(norm.len(), 95);
        assert!(norm.starts_with("AB:AB:"));
        assert_eq!(normalize_fingerprint(&norm).unwrap(), norm);
        assert!(normalize_fingerprint("abc").is_err());
        assert!(normalize_fingerprint(&"zz".repeat(32)).is_err());
    }
}
