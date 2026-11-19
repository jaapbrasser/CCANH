//! Simulated AXIS cameras for demo mode (SPEC §12.2).
//!
//! Behaviour is deterministic: every random choice comes from a PRNG seeded by
//! the camera's host, and time-based behaviour (flapping) is measured from when
//! the provider was created using Tokio's clock, so tests can pause time.
//! No method ever opens a network connection.

pub mod catalog;
pub mod models;

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use self::catalog::Scenario;
use self::models::{ModelKey, ALL_MODELS};
use super::{
    CameraCapabilities, CameraProvider, CameraStatus, DeviceInfo, MediaSource, NetworkInfo,
    ProbeResult, ProviderError, ProviderTarget, StreamProfile, StreamProfileId, VideoCodec,
};

/// How long a simulated unreachable camera takes to fail (like a connect timeout).
pub const OFFLINE_DELAY: Duration = Duration::from_millis(800);

/// SplitMix64: tiny, fast, good enough for simulated jitter.
#[derive(Debug)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        let span = u64::from(hi.saturating_sub(lo)).max(1);
        lo + u32::try_from(self.next() % span).unwrap_or(0)
    }
}

/// FNV-1a, for stable seeds from host names.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b.to_ascii_lowercase())).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The password demo cameras reject.
pub const DEMO_WRONG_PASSWORD: &str = "wrong";

#[derive(Debug)]
pub struct DemoProvider {
    host: String,
    name: Option<&'static str>,
    model: ModelKey,
    scenario: Scenario,
    seed: u64,
    rng: Mutex<SplitMix64>,
    created: Instant,
}

impl DemoProvider {
    /// A provider for `target`. Hosts from the demo fleet get their catalog model
    /// and scenario; any other host is a healthy camera with a model chosen from
    /// the host's hash.
    pub fn for_target(target: &ProviderTarget) -> Self {
        let seed = fnv1a(&target.host);
        let (name, model, mut scenario) = match catalog::find(&target.host) {
            Some(c) => (Some(c.name), c.model, c.scenario),
            None => {
                let index = usize::try_from(seed % ALL_MODELS.len() as u64).unwrap_or(0);
                (None, ALL_MODELS[index], Scenario::Healthy)
            }
        };
        // Demo cameras accept any credentials except the password "wrong", so
        // rejected credentials can be tried out (docs/demo.md).
        if target.credentials.as_ref().is_some_and(|c| {
            crate::secret::ExposeSecret::expose_secret(&c.password) == DEMO_WRONG_PASSWORD
        }) {
            scenario = Scenario::AuthFailure;
        }
        Self::new(&target.host, name, model, scenario, seed)
    }

    pub fn new(
        host: &str,
        name: Option<&'static str>,
        model: ModelKey,
        scenario: Scenario,
        seed: u64,
    ) -> Self {
        Self {
            host: host.to_owned(),
            name,
            model,
            scenario,
            seed,
            rng: Mutex::new(SplitMix64(seed)),
            created: Instant::now(),
        }
    }

    fn jitter(&self, lo: u32, hi: u32) -> u32 {
        self.rng
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .range(lo, hi)
    }

    fn is_online_now(&self) -> bool {
        match self.scenario {
            Scenario::Offline => false,
            Scenario::Flapping { period_secs } => {
                let elapsed = self.created.elapsed().as_secs();
                (elapsed / period_secs.max(1)).is_multiple_of(2)
            }
            Scenario::Healthy | Scenario::Slow | Scenario::AuthFailure => true,
        }
    }

    /// Simulate a request: wait a realistic time, then succeed or fail per the
    /// scenario. Returns the simulated response time. `authenticated` mirrors
    /// VAPIX: identity and status calls are anonymous, so a camera with wrong
    /// credentials still answers them (like the AXIS provider).
    async fn request_as(&self, authenticated: bool) -> Result<u32, ProviderError> {
        if !self.is_online_now() {
            tokio::time::sleep(OFFLINE_DELAY).await;
            return Err(ProviderError::Unreachable(format!(
                "no response from {}",
                self.host
            )));
        }
        let ms = match self.scenario {
            Scenario::Slow => self.jitter(450, 900),
            _ => self.jitter(15, 90),
        };
        tokio::time::sleep(Duration::from_millis(u64::from(ms))).await;
        if authenticated && self.scenario == Scenario::AuthFailure {
            return Err(ProviderError::AuthFailed(
                "camera rejected the configured credentials".into(),
            ));
        }
        Ok(ms)
    }

    async fn request(&self) -> Result<u32, ProviderError> {
        self.request_as(false).await
    }

    fn mac(&self) -> String {
        // AXIS OUI with a device part derived from the seed.
        let b = self.seed.to_be_bytes();
        format!("AC:CC:8E:{:02X}:{:02X}:{:02X}", b[5], b[6], b[7])
    }

    fn device_info(&self) -> DeviceInfo {
        let profile = self.model.profile();
        let mac = self.mac();
        let serial = mac.replace(':', "");
        let is_ip = self.host.parse::<std::net::IpAddr>().is_ok();
        DeviceInfo {
            manufacturer: Some("AXIS".to_owned()),
            model: Some(profile.model.to_owned()),
            firmware_version: Some(profile.firmware.to_owned()),
            serial_number: Some(serial.clone()),
            mac_address: Some(mac),
            device_name: Some(
                self.name
                    .map_or_else(|| profile.model.to_owned(), str::to_owned),
            ),
            hostname: Some(format!("axis-{}", serial.to_lowercase())),
            network: profile
                .supports("api.network-settings")
                .then(|| NetworkInfo {
                    ipv4_address: is_ip.then(|| self.host.clone()),
                    subnet_mask: is_ip.then(|| "255.255.255.0".to_owned()),
                    gateway: None,
                    dhcp: Some(false),
                }),
        }
    }
}

#[async_trait]
impl CameraProvider for DemoProvider {
    async fn probe(&self) -> Result<ProbeResult, ProviderError> {
        let response_time_ms = self.request().await?;
        Ok(ProbeResult {
            response_time_ms,
            device: self.device_info(),
        })
    }

    async fn get_device_info(&self) -> Result<DeviceInfo, ProviderError> {
        self.request().await?;
        Ok(self.device_info())
    }

    async fn get_status(&self) -> Result<CameraStatus, ProviderError> {
        let response_time_ms = self.request().await?;
        Ok(CameraStatus {
            response_time_ms,
            uptime_seconds: Some(86_400 * 12 + self.created.elapsed().as_secs()),
        })
    }

    async fn get_capabilities(&self) -> Result<CameraCapabilities, ProviderError> {
        self.request().await?;
        let profile = self.model.profile();
        if !profile.supports("api.api-discovery") {
            // Older firmware: capabilities are inferred, not discovered. Still a
            // successful answer, just a less detailed one.
            return Ok(CameraCapabilities {
                items: profile
                    .capabilities()
                    .into_iter()
                    .map(|mut c| {
                        if !c.supported {
                            c.detail = Some("Not reported by this firmware".into());
                        }
                        c
                    })
                    .collect(),
            });
        }
        Ok(CameraCapabilities {
            items: profile.capabilities(),
        })
    }

    async fn get_stream_info(&self) -> Result<Vec<StreamProfile>, ProviderError> {
        self.request_as(true).await?;
        Ok(self.model.profile().stream_profiles())
    }

    async fn open_stream(
        &self,
        profile: &StreamProfileId,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError> {
        // Same scenario rules as other calls: offline cameras can't stream and
        // wrong credentials are rejected (RTSP needs authentication).
        self.request_as(true).await?;
        let model = self.model.profile();
        let Some(sp) = model
            .stream_profiles()
            .into_iter()
            .find(|p| p.id == *profile)
        else {
            // Asking for a profile that doesn't exist is permanent, not transient.
            return Err(ProviderError::unsupported(format!(
                "stream profile \"{profile}\" (not configured on this camera)"
            )));
        };
        if sp.codec != Some(VideoCodec::H264) {
            return Err(ProviderError::unsupported(
                "live view of H.265/MJPEG profiles (only H.264 is supported)",
            ));
        }
        let pattern = usize::try_from(self.seed % 4).unwrap_or(0);
        let clip = crate::streaming::demo::clip(pattern, &sp.id.0)
            .ok_or_else(|| ProviderError::Internal("demo clip missing".into()))?;
        Ok(Box::new(crate::streaming::demo::DemoMediaSource::new(
            clip, cancel,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ErrorCategory;

    fn provider(host: &str) -> DemoProvider {
        DemoProvider::for_target(&ProviderTarget::simple(host, 443, true))
    }

    #[tokio::test(start_paused = true)]
    async fn same_host_is_deterministic() {
        let a = provider("192.0.2.11");
        let b = provider("192.0.2.11");
        for _ in 0..5 {
            assert_eq!(a.probe().await.unwrap(), b.probe().await.unwrap());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn offline_camera_is_unreachable_after_a_delay() {
        let p = provider("192.0.2.34");
        let start = Instant::now();
        let err = p.probe().await.unwrap_err();
        assert_eq!(err.category(), ErrorCategory::Unreachable);
        assert!(start.elapsed() >= OFFLINE_DELAY);
    }

    #[tokio::test(start_paused = true)]
    async fn auth_failure_camera_answers_anonymous_calls_but_rejects_credentials() {
        let p = provider("198.51.100.24");
        assert!(p.get_status().await.is_ok(), "status is anonymous");
        let err = p.get_stream_info().await.unwrap_err();
        assert_eq!(err.category(), ErrorCategory::AuthFailed);
    }

    #[tokio::test(start_paused = true)]
    async fn flapping_camera_alternates() {
        let p = provider("198.51.100.13");
        assert!(p.get_status().await.is_ok());
        tokio::time::advance(Duration::from_secs(46)).await;
        assert!(p.get_status().await.is_err());
        tokio::time::advance(Duration::from_secs(45)).await;
        assert!(p.get_status().await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn slow_camera_is_slow() {
        let fast = provider("192.0.2.31").get_status().await.unwrap();
        let slow = provider("192.0.2.33").get_status().await.unwrap();
        assert!(fast.response_time_ms < 100);
        assert!(slow.response_time_ms >= 450);
    }

    #[tokio::test(start_paused = true)]
    async fn identity_reflects_model_and_fleet_name() {
        let info = provider("192.0.2.11").get_device_info().await.unwrap();
        assert_eq!(info.model.as_deref(), Some("AXIS Q6135-LE"));
        assert_eq!(info.device_name.as_deref(), Some("A1 Crane 1 – Hook"));
        assert_eq!(
            info.serial_number.unwrap(),
            info.mac_address.unwrap().replace(':', "")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn streams_h264_profiles_and_rejects_others() {
        use crate::streaming::media::MediaEvent;
        let p = provider("192.0.2.21"); // AXIS Q1656: main, main-h265, grid
        let mut src = p
            .open_stream(&StreamProfileId("grid".into()), CancellationToken::new())
            .await
            .unwrap();
        let Some(MediaEvent::Params(params)) = src.next().await.unwrap() else {
            panic!()
        };
        assert_eq!(params.width, 640);
        let Some(MediaEvent::Frame(f)) = src.next().await.unwrap() else {
            panic!()
        };
        assert!(f.keyframe);

        let h265 = p
            .open_stream(
                &StreamProfileId("main-h265".into()),
                CancellationToken::new(),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(h265.category(), ErrorCategory::Unsupported);
        assert!(p
            .open_stream(&StreamProfileId("nope".into()), CancellationToken::new())
            .await
            .is_err());
        // Offline cameras can't stream.
        let offline = provider("192.0.2.34")
            .open_stream(&StreamProfileId("main".into()), CancellationToken::new())
            .await
            .err()
            .unwrap();
        assert_eq!(offline.category(), ErrorCategory::Unreachable);
    }
}
