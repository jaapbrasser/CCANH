//! Camera provider abstraction (SPEC §4.3).
//!
//! The application core only talks to cameras through [`CameraProvider`]. One
//! provider instance exists per registered camera, built by [`ProviderFactory`]
//! from the camera record and the run mode. Demo and production share every code
//! path above this trait (SPEC §1.1 principle 6).
//!
//! `Unsupported` is a normal result, not a failure: callers record it and carry on.

pub mod axis;
#[cfg(test)]
pub(crate) mod contract;
pub mod demo;
mod factory;

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

pub use factory::{ProviderFactory, ProviderTarget};

/// Error categories shared by the API, UI and retry policy (SPEC §10.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ErrorCategory {
    Unreachable,
    Timeout,
    AuthFailed,
    TlsError,
    Unsupported,
    ProtocolError,
    RateLimited,
    Internal,
}

impl ErrorCategory {
    /// Whether retrying soon is reasonable. Auth failures are never retried in a
    /// tight loop: repeated attempts can lock out camera accounts (SPEC §10.4).
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ErrorCategory::Unreachable | ErrorCategory::Timeout | ErrorCategory::RateLimited
        )
    }
}

/// Provider errors. Messages are short and must never contain credentials.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("not supported: {feature}")]
    Unsupported { feature: String },
    #[error("unreachable: {0}")]
    Unreachable(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    #[error("TLS error: {0}")]
    Tls(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ProviderError {
    pub fn unsupported(feature: impl Into<String>) -> Self {
        ProviderError::Unsupported {
            feature: feature.into(),
        }
    }

    pub fn category(&self) -> ErrorCategory {
        match self {
            ProviderError::Unsupported { .. } => ErrorCategory::Unsupported,
            ProviderError::Unreachable(_) => ErrorCategory::Unreachable,
            ProviderError::Timeout(_) => ErrorCategory::Timeout,
            ProviderError::AuthFailed(_) => ErrorCategory::AuthFailed,
            ProviderError::Tls(_) => ErrorCategory::TlsError,
            ProviderError::Protocol(_) => ErrorCategory::ProtocolError,
            ProviderError::RateLimited(_) => ErrorCategory::RateLimited,
            ProviderError::Internal(_) => ErrorCategory::Internal,
        }
    }
}

/// Basic network information, where the camera reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct NetworkInfo {
    pub ipv4_address: Option<String>,
    pub subnet_mask: Option<String>,
    pub gateway: Option<String>,
    pub dhcp: Option<bool>,
}

/// Device identification. Every field is optional: models differ.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DeviceInfo {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub mac_address: Option<String>,
    pub device_name: Option<String>,
    pub hostname: Option<String>,
    pub network: Option<NetworkInfo>,
}

/// Result of a reachability + identity probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub response_time_ms: u32,
    pub device: DeviceInfo,
}

/// Point-in-time status from the camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraStatus {
    pub response_time_ms: u32,
    pub uptime_seconds: Option<u64>,
}

/// Capability groups drive the sections of the camera detail page (SPEC §6.3):
/// a new group becomes a new section without a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGroup {
    Device,
    Network,
    Video,
    Audio,
    Ptz,
    Io,
    Events,
    Storage,
    Analytics,
}

/// One capability (an API or feature) and whether this camera supports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Capability {
    /// Stable key, e.g. `video.h264` or `api.basic-device-info`.
    pub key: String,
    pub group: CapabilityGroup,
    pub label: String,
    pub supported: bool,
    /// API version, where applicable.
    pub version: Option<String>,
    /// Short extra information, e.g. why it's unsupported.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CameraCapabilities {
    pub items: Vec<Capability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264,
    H265,
    Mjpeg,
}

/// Identifies a stream profile on a camera, e.g. `"main"` or `"grid"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct StreamProfileId(pub String);

impl fmt::Display for StreamProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stream profile. AXIS profiles only carry the parameters an admin saved in
/// them, so codec and resolution may be unknown (the camera default applies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StreamProfile {
    /// Profile name used in stream requests. Empty for the camera's default stream.
    #[schema(value_type = String)]
    pub id: StreamProfileId,
    pub name: String,
    pub codec: Option<VideoCodec>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub bitrate_kbps: Option<u32>,
}

/// An upstream media source: codec parameters, then timestamped access units.
/// See [`crate::streaming::media`].
pub use crate::streaming::media::MediaSource;

/// The interface every camera integration implements (SPEC §4.3).
///
/// `async_trait` is used because providers are held as `dyn CameraProvider`.
/// Implementations must time out every network call and never block.
#[async_trait]
pub trait CameraProvider: Send + Sync {
    /// Reachability and identity.
    async fn probe(&self) -> Result<ProbeResult, ProviderError>;
    async fn get_device_info(&self) -> Result<DeviceInfo, ProviderError>;
    async fn get_status(&self) -> Result<CameraStatus, ProviderError>;
    async fn get_capabilities(&self) -> Result<CameraCapabilities, ProviderError>;
    async fn get_stream_info(&self) -> Result<Vec<StreamProfile>, ProviderError>;

    /// Open an upstream media source. Called ONLY by the stream manager.
    async fn open_stream(
        &self,
        profile: &StreamProfileId,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError>;

    /// Restart the camera (issue #33). Returns once the camera has accepted the
    /// request; it then goes offline while it reboots, which the status
    /// monitor sees as usual.
    async fn restart(&self) -> Result<(), ProviderError>;

    /// SHA-256 fingerprint of the certificate the camera presented on the last
    /// HTTPS connection, for "test connection" and pinning. `None` when not
    /// applicable (plain HTTP, simulated cameras).
    fn tls_fingerprint(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_map_and_auth_is_not_retryable() {
        assert_eq!(
            ProviderError::unsupported("ptz").category(),
            ErrorCategory::Unsupported
        );
        assert_eq!(
            ProviderError::AuthFailed("401".into()).category(),
            ErrorCategory::AuthFailed
        );
        assert!(!ErrorCategory::AuthFailed.is_retryable());
        assert!(ErrorCategory::Timeout.is_retryable());
    }

    #[test]
    fn category_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&ErrorCategory::AuthFailed).unwrap(),
            "\"auth_failed\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCategory::TlsError).unwrap(),
            "\"tls_error\""
        );
    }
}
