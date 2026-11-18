//! Camera API models and input validation.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::providers::{Capability, ErrorCategory, NetworkInfo, StreamProfile};

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
}

/// Everything known about a camera (SPEC §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CameraDetail {
    #[serde(flatten)]
    pub summary: CameraSummary,
    pub tls_mode: TlsMode,
    pub tls_pinned_sha256: Option<String>,
    pub serial_number: Option<String>,
    pub mac_address: Option<String>,
    pub device_name: Option<String>,
    pub device_hostname: Option<String>,
    pub network: Option<NetworkInfo>,
    /// Supported and unsupported capabilities. Empty until the first successful probe.
    pub capabilities: Vec<Capability>,
    pub stream_profiles: Vec<StreamProfile>,
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

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewCamera {
    pub name: String,
    /// IP address or hostname, without scheme, port or path.
    pub host: String,
    /// Defaults to 443 for HTTPS, 80 for HTTP.
    pub port: Option<u16>,
    #[serde(default)]
    pub scheme: Scheme,
    #[serde(default)]
    pub tls_mode: TlsMode,
    /// SHA-256 certificate fingerprint (hex, colons optional). Required for `pinned`.
    pub tls_pinned_sha256: Option<String>,
    #[serde(default = "default_true")]
    pub monitoring_enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Partial update. Omitted fields are unchanged. Send `tls_pinned_sha256: null`
/// together with a non-pinned `tls_mode` to clear a pin.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CameraPatch {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub scheme: Option<Scheme>,
    pub tls_mode: Option<TlsMode>,
    pub tls_pinned_sha256: Option<String>,
    pub monitoring_enabled: Option<bool>,
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
