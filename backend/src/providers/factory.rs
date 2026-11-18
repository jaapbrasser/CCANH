//! Builds the right provider for a camera record and run mode (SPEC §4.3).

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::demo::DemoProvider;
use super::{
    CameraCapabilities, CameraProvider, CameraStatus, DeviceInfo, MediaSource, ProbeResult,
    ProviderError, StreamProfile, StreamProfileId,
};
use crate::config::Mode;

/// What a provider needs to know about the camera it talks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTarget {
    pub host: String,
    pub port: u16,
    pub https: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderFactory {
    mode: Mode,
}

impl ProviderFactory {
    pub fn new(mode: Mode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Build a provider for `target`.
    ///
    /// In demo mode this is *always* the [`DemoProvider`], including for cameras an
    /// admin adds by hand, so demo mode never opens an outbound connection (SPEC
    /// §12.1).
    pub fn create(&self, target: &ProviderTarget) -> Arc<dyn CameraProvider> {
        match self.mode {
            Mode::Demo => Arc::new(DemoProvider::for_target(target)),
            Mode::Production => Arc::new(PendingAxisProvider),
        }
    }
}

/// Production placeholder until the AXIS provider lands in Phase 4: every call
/// reports `Unsupported`, which the monitor treats as "status unknown", not as a
/// failure. Removed when `AxisProvider` exists.
struct PendingAxisProvider;

const PENDING: &str = "AXIS integration (arrives in Phase 4)";

#[async_trait]
impl CameraProvider for PendingAxisProvider {
    async fn probe(&self) -> Result<ProbeResult, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
    async fn get_device_info(&self) -> Result<DeviceInfo, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
    async fn get_status(&self) -> Result<CameraStatus, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
    async fn get_capabilities(&self) -> Result<CameraCapabilities, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
    async fn get_stream_info(&self) -> Result<Vec<StreamProfile>, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
    async fn open_stream(
        &self,
        _profile: &StreamProfileId,
        _cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError> {
        Err(ProviderError::unsupported(PENDING))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ErrorCategory;

    fn target(host: &str) -> ProviderTarget {
        ProviderTarget {
            host: host.into(),
            port: 443,
            https: true,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn demo_mode_always_uses_demo_provider_even_for_real_looking_hosts() {
        let factory = ProviderFactory::new(Mode::Demo);
        // A host that is not in the demo catalog still gets a simulated camera.
        let provider = factory.create(&target("10.1.2.3"));
        let probe = provider.probe().await.unwrap();
        assert_eq!(probe.device.manufacturer.as_deref(), Some("AXIS"));
    }

    #[tokio::test]
    async fn production_reports_unsupported_until_axis_provider_exists() {
        let factory = ProviderFactory::new(Mode::Production);
        let err = factory
            .create(&target("camera.local"))
            .probe()
            .await
            .unwrap_err();
        assert_eq!(err.category(), ErrorCategory::Unsupported);
    }
}
