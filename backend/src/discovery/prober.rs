//! Probing one address (SPEC §5.1): a quick TCP check on the HTTPS and HTTP
//! ports, then at most one identification request to a responsive host.
//!
//! Production uses real sockets and the anonymous VAPIX
//! `basicdeviceinfo.cgi getAllUnrestrictedProperties` call (the provider's
//! `probe`, see docs/vapix.md). Demo mode never opens a socket: it answers
//! from the simulated fleet plus a few unregistered demo devices.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::cameras::Scheme;
use crate::config::Mode;
use crate::providers::axis::tls::TlsPolicy;
use crate::providers::axis::AxisProvider;
use crate::providers::demo::{catalog, DemoProvider};
use crate::providers::{CameraProvider, DeviceInfo, ProviderTarget};

#[derive(Debug, Clone, Copy)]
pub struct ProbeOptions {
    pub connect_timeout: Duration,
    pub http_timeout: Duration,
}

/// An identified device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identified {
    pub port: u16,
    pub scheme: Scheme,
    pub device: DeviceInfo,
    pub response_time_ms: u32,
    pub tls_fingerprint_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Neither port accepted a connection.
    NoService,
    /// A web server answered but didn't identify as an AXIS device.
    Unidentified,
    Found(Box<Identified>),
}

#[async_trait]
pub trait HostProber: Send + Sync {
    async fn probe(&self, ip: Ipv4Addr, opts: &ProbeOptions) -> ProbeOutcome;
}

/// The prober for this mode: demo mode never touches the network (SPEC §12.1).
pub fn for_mode(mode: Mode) -> Arc<dyn HostProber> {
    match mode {
        Mode::Demo => Arc::new(DemoProber),
        Mode::Production => Arc::new(AxisProber::default()),
    }
}

async fn port_open(ip: Ipv4Addr, port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(
            timeout,
            tokio::net::TcpStream::connect(SocketAddr::from((ip, port)))
        )
        .await,
        Ok(Ok(_))
    )
}

async fn identify(
    target: &ProviderTarget,
    provider: &dyn CameraProvider,
    scheme: Scheme,
) -> ProbeOutcome {
    match provider.probe().await {
        Ok(p) => ProbeOutcome::Found(Box::new(Identified {
            port: target.port,
            scheme,
            device: p.device,
            response_time_ms: p.response_time_ms,
            tls_fingerprint_sha256: provider.tls_fingerprint(),
        })),
        Err(_) => ProbeOutcome::Unidentified,
    }
}

/// Real network scanning for AXIS devices.
pub struct AxisProber {
    pub https_port: u16,
    pub http_port: u16,
}

impl Default for AxisProber {
    fn default() -> Self {
        Self {
            https_port: 443,
            http_port: 80,
        }
    }
}

#[async_trait]
impl HostProber for AxisProber {
    async fn probe(&self, ip: Ipv4Addr, opts: &ProbeOptions) -> ProbeOutcome {
        // Both ports at once, so a silent address costs one timeout, not two.
        let (https, http) = tokio::join!(
            port_open(ip, self.https_port, opts.connect_timeout),
            port_open(ip, self.http_port, opts.connect_timeout)
        );
        let (port, scheme) = match (https, http) {
            (true, _) => (self.https_port, Scheme::Https),
            (false, true) => (self.http_port, Scheme::Http),
            (false, false) => return ProbeOutcome::NoService,
        };
        // The certificate can't be verified yet; only public identification is
        // read, and the fingerprint seen is shown for the admin to pin.
        let target = ProviderTarget {
            host: ip.to_string(),
            port,
            https: scheme == Scheme::Https,
            tls: TlsPolicy::InsecureSkipVerify,
            credentials: None,
            allow_basic_over_http: false,
        };
        let provider = AxisProvider::new(&target, opts.http_timeout);
        identify(&target, &provider, scheme).await
    }
}

/// Unregistered devices that demo scans can find, besides the demo fleet.
pub const DEMO_EXTRA_HOSTS: [&str; 12] = [
    "192.0.2.41",
    "192.0.2.42",
    "192.0.2.43",
    "192.0.2.44",
    "192.0.2.45",
    "192.0.2.46",
    "198.51.100.41",
    "198.51.100.42",
    "198.51.100.43",
    "198.51.100.44",
    "198.51.100.45",
    "198.51.100.46",
];

/// Extra demo devices that only answer on plain HTTP, like older cameras on
/// site that use Basic authentication over HTTP (issue #17).
pub const DEMO_HTTP_HOSTS: [&str; 4] =
    ["192.0.2.42", "192.0.2.44", "198.51.100.42", "198.51.100.44"];

/// A web server in the demo ranges that isn't a camera (shows "unidentified").
const DEMO_OTHER_WEB_SERVERS: [&str; 2] = ["192.0.2.1", "198.51.100.1"];

/// Simulated scanning with realistic timing, deterministic per address.
pub struct DemoProber;

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[async_trait]
impl HostProber for DemoProber {
    async fn probe(&self, ip: Ipv4Addr, opts: &ProbeOptions) -> ProbeOutcome {
        let host = ip.to_string();
        let jitter = fnv(&host) % 100;
        let is_camera = catalog::find(&host).is_some() || DEMO_EXTRA_HOSTS.contains(&host.as_str());
        if !is_camera && !DEMO_OTHER_WEB_SERVERS.contains(&host.as_str()) {
            // A silent address: wait out (a tenth of) the connect timeout.
            tokio::time::sleep(opts.connect_timeout / 10 + Duration::from_millis(jitter)).await;
            return ProbeOutcome::NoService;
        }
        tokio::time::sleep(Duration::from_millis(20 + jitter)).await;
        if !is_camera {
            return ProbeOutcome::Unidentified;
        }
        let https = !DEMO_HTTP_HOSTS.contains(&host.as_str());
        let (port, scheme) = if https {
            (443, Scheme::Https)
        } else {
            (80, Scheme::Http)
        };
        let target = ProviderTarget::simple(host, port, https);
        // Offline demo cameras don't answer, like real ones.
        let provider = DemoProvider::for_target(&target);
        match tokio::time::timeout(opts.http_timeout, identify(&target, &provider, scheme)).await {
            Ok(ProbeOutcome::Found(f)) => ProbeOutcome::Found(f),
            Ok(_) => ProbeOutcome::NoService,
            Err(_) => ProbeOutcome::Unidentified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ProbeOptions {
        ProbeOptions {
            connect_timeout: Duration::from_millis(300),
            http_timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn demo_finds_fleet_and_extra_devices_only() {
        let p = DemoProber;
        let found = |o: ProbeOutcome| matches!(o, ProbeOutcome::Found(_));
        assert!(
            found(p.probe("192.0.2.11".parse().unwrap(), &opts()).await),
            "registered fleet camera"
        );
        assert!(
            found(p.probe("192.0.2.41".parse().unwrap(), &opts()).await),
            "unregistered demo device"
        );
        assert_eq!(
            p.probe("192.0.2.200".parse().unwrap(), &opts()).await,
            ProbeOutcome::NoService
        );
        assert_eq!(
            p.probe("192.0.2.1".parse().unwrap(), &opts()).await,
            ProbeOutcome::Unidentified
        );
        // An always-offline fleet camera doesn't show up.
        assert!(!found(
            p.probe("192.0.2.34".parse().unwrap(), &opts()).await
        ));
    }

    fn closed_port() -> u16 {
        // Bound then dropped: nothing listens there.
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn axis_prober_reports_closed_ports_quickly() {
        let started = std::time::Instant::now();
        let o = AxisProber {
            https_port: closed_port(),
            http_port: closed_port(),
        }
        .probe("127.0.0.1".parse().unwrap(), &opts())
        .await;
        assert_eq!(o, ProbeOutcome::NoService);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn axis_prober_identifies_with_one_anonymous_request() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let camera = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/axis-cgi/basicdeviceinfo.cgi"))
            .and(body_partial_json(
                serde_json::json!({ "method": "getAllUnrestrictedProperties" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/vapix/basicdeviceinfo_unrestricted.json"
                )),
                "application/json",
            ))
            // Verified when the server drops: exactly one request per host.
            .expect(1)
            .mount(&camera)
            .await;
        let prober = AxisProber {
            https_port: closed_port(),
            http_port: camera.address().port(),
        };
        let o = prober.probe("127.0.0.1".parse().unwrap(), &opts()).await;
        let ProbeOutcome::Found(f) = o else {
            panic!("{o:?}")
        };
        assert_eq!(f.scheme, Scheme::Http);
        assert_eq!(f.port, camera.address().port());
        assert!(f.device.model.is_some());
    }

    #[tokio::test]
    async fn axis_prober_reports_other_web_servers_as_unidentified() {
        use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let o = AxisProber {
            https_port: closed_port(),
            http_port: server.address().port(),
        }
        .probe("127.0.0.1".parse().unwrap(), &opts())
        .await;
        assert_eq!(o, ProbeOutcome::Unidentified);
    }
}
