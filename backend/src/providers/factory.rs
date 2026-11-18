//! Builds the right provider for a camera record and run mode (SPEC §4.3).

use std::sync::Arc;
use std::time::Duration;

use super::axis::tls::TlsPolicy;
use super::axis::AxisProvider;
use super::demo::DemoProvider;
use super::CameraProvider;
use crate::config::Mode;
use crate::credentials::Credentials;

/// What a provider needs to know about the camera it talks to. `Debug` never
/// shows the password (see [`Credentials`]).
#[derive(Debug, Clone)]
pub struct ProviderTarget {
    pub host: String,
    pub port: u16,
    pub https: bool,
    pub tls: TlsPolicy,
    pub credentials: Option<Credentials>,
    /// Allow Basic authentication over plain HTTP (off unless an admin opts in).
    pub allow_basic_over_http: bool,
}

impl ProviderTarget {
    /// A target with no credentials and strict TLS; mostly for tests.
    pub fn simple(host: impl Into<String>, port: u16, https: bool) -> Self {
        Self {
            host: host.into(),
            port,
            https,
            tls: TlsPolicy::Strict,
            credentials: None,
            allow_basic_over_http: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderFactory {
    mode: Mode,
    timeout: Duration,
}

impl ProviderFactory {
    /// `timeout` bounds each individual camera request.
    pub fn new(mode: Mode, timeout: Duration) -> Self {
        Self { mode, timeout }
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
            Mode::Production => Arc::new(AxisProvider::new(target, self.timeout)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ErrorCategory;

    fn target(host: &str) -> ProviderTarget {
        ProviderTarget::simple(host, 443, true)
    }

    #[tokio::test(start_paused = true)]
    async fn demo_mode_always_uses_demo_provider_even_for_real_looking_hosts() {
        let factory = ProviderFactory::new(Mode::Demo, Duration::from_secs(5));
        // A host that is not in the demo catalog still gets a simulated camera.
        let provider = factory.create(&target("10.1.2.3"));
        let probe = provider.probe().await.unwrap();
        assert_eq!(probe.device.manufacturer.as_deref(), Some("AXIS"));
    }

    #[tokio::test]
    async fn production_uses_axis_provider() {
        // Nothing listens on this port: the AXIS provider reports it unreachable.
        let factory = ProviderFactory::new(Mode::Production, Duration::from_secs(2));
        let err = factory
            .create(&ProviderTarget::simple("127.0.0.1", 1, false))
            .probe()
            .await
            .unwrap_err();
        assert_eq!(err.category(), ErrorCategory::Unreachable);
    }
}
