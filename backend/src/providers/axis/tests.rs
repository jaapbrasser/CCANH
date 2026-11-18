//! AxisProvider against a mock VAPIX server (`wiremock`) loaded with the
//! fixtures in `backend/tests/fixtures/vapix/` (SPEC §15.1).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::digest::{digest_authorization, Algorithm, DigestChallenge};
use super::tls::TlsPolicy;
use super::*;
use crate::credentials::Credentials;
use crate::providers::contract;
use crate::secret::SecretString;

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vapix/",
            $name
        ))
    };
}

const USER: &str = "viewer";
const PASS: &str = "correct-camera-password";
const REALM: &str = "AXIS_ACCC8E000000";
const NONCE: &str = "0000a1b2c3d4e5f60708090a0b0c0d0e";

fn json_reply(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

/// Responds like an AXIS endpoint protected by Digest authentication: a 401
/// challenge without (or with wrong) credentials, `ok` when the Digest response
/// verifies against `password`.
struct DigestGate {
    ok: ResponseTemplate,
    password: &'static str,
    basic_only: bool,
}

impl DigestGate {
    fn new(ok: ResponseTemplate) -> Self {
        Self {
            ok,
            password: PASS,
            basic_only: false,
        }
    }

    fn challenge(&self) -> ResponseTemplate {
        let header = if self.basic_only {
            format!("Basic realm=\"{REALM}\"")
        } else {
            format!("Digest realm=\"{REALM}\", nonce=\"{NONCE}\", algorithm=MD5, qop=\"auth\"")
        };
        ResponseTemplate::new(401).insert_header("WWW-Authenticate", header.as_str())
    }
}

fn param<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let start = header.find(&needle)? + needle.len();
    let rest = &header[start..];
    if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next()
    } else {
        rest.split(',').next().map(str::trim)
    }
}

impl Respond for DigestGate {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let Some(auth) = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
        else {
            return self.challenge();
        };
        if self.basic_only {
            use base64::Engine;
            let expected = base64::engine::general_purpose::STANDARD
                .encode(format!("{USER}:{}", self.password));
            return if auth == format!("Basic {expected}") {
                self.ok.clone()
            } else {
                self.challenge()
            };
        }
        let (Some(user), Some(uri), Some(nc), Some(cnonce), Some(response)) = (
            param(auth, "username"),
            param(auth, "uri"),
            param(auth, "nc"),
            param(auth, "cnonce"),
            param(auth, "response"),
        ) else {
            return self.challenge();
        };
        let ch = DigestChallenge {
            realm: REALM.into(),
            nonce: NONCE.into(),
            opaque: None,
            algorithm: Algorithm::Md5,
            qop_auth: true,
        };
        let nc = u32::from_str_radix(nc, 16).unwrap_or(0);
        let expected = digest_authorization(&ch, USER, self.password, "POST", uri, nc, cnonce);
        if user == USER && param(&expected, "response") == Some(response) {
            self.ok.clone()
        } else {
            self.challenge()
        }
    }
}

/// Fails the test if a request carries an Authorization header.
struct NoAuth;
impl wiremock::Match for NoAuth {
    fn matches(&self, req: &Request) -> bool {
        !req.headers.contains_key("authorization")
    }
}

fn creds(password: &str) -> Option<Credentials> {
    Some(Credentials {
        username: USER.into(),
        password: SecretString::from(password.to_owned()),
    })
}

fn provider_for(server: &MockServer, credentials: Option<Credentials>) -> AxisProvider {
    let addr = server.address();
    AxisProvider::new(
        &ProviderTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
            https: false,
            tls: TlsPolicy::Strict,
            credentials,
            allow_basic_over_http: false,
        },
        Duration::from_secs(2),
    )
}

async fn mount_camera(server: &MockServer, full_api_list: bool) {
    Mock::given(method("POST"))
        .and(path(BASIC_DEVICE_INFO))
        .and(body_partial_json(
            json!({ "method": "getAllUnrestrictedProperties" }),
        ))
        .and(NoAuth)
        .respond_with(json_reply(fixture!("basicdeviceinfo_unrestricted.json")))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(SYSTEM_READY))
        .and(NoAuth)
        .respond_with(json_reply(fixture!("systemready.json")))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(API_DISCOVERY))
        .and(NoAuth)
        .respond_with(json_reply(if full_api_list {
            fixture!("apidiscovery_getapilist_full.json")
        } else {
            fixture!("apidiscovery_getapilist.json")
        }))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(STREAM_PROFILES))
        .respond_with(DigestGate::new(json_reply(fixture!(
            "streamprofile_list.json"
        ))))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path(NETWORK_SETTINGS))
        .respond_with(DigestGate::new(json_reply(fixture!(
            "network_getnetworkinfo.json"
        ))))
        .mount(server)
        .await;
}

#[tokio::test]
async fn probe_is_anonymous_and_maps_identity() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let probe = provider_for(&server, creds(PASS)).probe().await.unwrap();
    assert_eq!(probe.device.manufacturer.as_deref(), Some("AXIS"));
    assert_eq!(probe.device.model.as_deref(), Some("AXIS P3265-LVE"));
    assert_eq!(probe.device.firmware_version.as_deref(), Some("11.11.73"));
    assert_eq!(probe.device.serial_number.as_deref(), Some("ACCC8E000000"));
    assert!(probe.response_time_ms > 0);
}

#[tokio::test]
async fn digest_authenticated_calls_work() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let p = provider_for(&server, creds(PASS));
    let profiles = p.get_stream_info().await.unwrap();
    assert_eq!(profiles.len(), 3);
    let hd = &profiles[0];
    assert_eq!(hd.id.0, "My full HD profile");
    assert_eq!(
        (hd.width, hd.height, hd.fps),
        (Some(1920), Some(1080), Some(30))
    );
    assert_eq!(hd.codec, Some(VideoCodec::H264));
    assert_eq!(
        profiles[2].width, None,
        "profiles without resolution keep it unknown"
    );

    // A second authenticated call reuses the cached challenge (nc increments).
    let info = p.get_device_info().await.unwrap();
    assert_eq!(info.mac_address.as_deref(), Some("AC:CC:8E:68:8E:C4"));
    assert_eq!(info.hostname.as_deref(), Some("somehostname"));
    let net = info.network.unwrap();
    assert_eq!(net.ipv4_address.as_deref(), Some("192.0.2.165"));
    assert_eq!(net.subnet_mask.as_deref(), Some("255.255.255.0"));
    assert_eq!(net.gateway.as_deref(), Some("192.0.2.1"));
    assert_eq!(net.dhcp, Some(true));
}

#[tokio::test]
async fn wrong_password_is_auth_failed_and_not_retried_in_a_loop() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let err = provider_for(&server, creds("wrong-password"))
        .get_stream_info()
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::AuthFailed);
    assert!(!err.to_string().contains("wrong-password"));
    // One unauthenticated request plus exactly one authenticated retry.
    let n = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == STREAM_PROFILES)
        .count();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn missing_credentials_are_reported() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let err = provider_for(&server, None)
        .get_stream_info()
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::AuthFailed);
    assert!(err.to_string().contains("none are configured"));
}

#[tokio::test]
async fn basic_over_http_is_refused_unless_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(STREAM_PROFILES))
        .respond_with(DigestGate {
            ok: json_reply(fixture!("streamprofile_list.json")),
            password: PASS,
            basic_only: true,
        })
        .mount(&server)
        .await;

    let err = provider_for(&server, creds(PASS))
        .get_stream_info()
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::AuthFailed);
    assert!(err.to_string().contains("Basic"), "{err}");

    let addr = server.address();
    let allowed = AxisProvider::new(
        &ProviderTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
            https: false,
            tls: TlsPolicy::Strict,
            credentials: creds(PASS),
            allow_basic_over_http: true,
        },
        Duration::from_secs(2),
    );
    assert_eq!(allowed.get_stream_info().await.unwrap().len(), 3);
}

#[tokio::test]
async fn network_info_needs_admin_and_is_best_effort() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    Mock::given(method("POST"))
        .and(path(NETWORK_SETTINGS))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(1)
        .mount(&server)
        .await;
    let info = provider_for(&server, creds(PASS))
        .get_device_info()
        .await
        .unwrap();
    assert_eq!(info.model.as_deref(), Some("AXIS P3265-LVE"));
    assert!(info.network.is_none());
}

#[tokio::test]
async fn status_uses_systemready_and_falls_back_on_old_firmware() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let s = provider_for(&server, None).get_status().await.unwrap();
    assert_eq!(s.uptime_seconds, Some(7800));

    let old = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(BASIC_DEVICE_INFO))
        .respond_with(json_reply(fixture!("basicdeviceinfo_unrestricted.json")))
        .mount(&old)
        .await;
    // No systemready.cgi → 404 → fallback.
    let s = provider_for(&old, None).get_status().await.unwrap();
    assert_eq!(s.uptime_seconds, None);
}

#[tokio::test]
async fn capabilities_from_api_discovery() {
    let server = MockServer::start().await;
    mount_camera(&server, true).await;
    let caps = provider_for(&server, None)
        .get_capabilities()
        .await
        .unwrap()
        .items;
    let get = |k: &str| caps.iter().find(|c| c.key == k);
    assert_eq!(get("api.ptz-control").unwrap().group, CapabilityGroup::Ptz);
    assert_eq!(
        get("api.mqtt-client").unwrap().group,
        CapabilityGroup::Events
    );
    assert_eq!(
        get("api.stream-profiles").unwrap().group,
        CapabilityGroup::Video
    );
    assert!(
        get("api.old-thing").is_none(),
        "deprecated APIs are skipped"
    );

    // Documented example list: highest version wins; stream-profiles missing.
    let server = MockServer::start().await;
    mount_camera(&server, false).await;
    let caps = provider_for(&server, None)
        .get_capabilities()
        .await
        .unwrap()
        .items;
    let disc = caps.iter().find(|c| c.key == "api.api-discovery").unwrap();
    assert_eq!(disc.version.as_deref(), Some("2.0"));
    let sp = caps
        .iter()
        .find(|c| c.key == "api.stream-profiles")
        .unwrap();
    assert!(!sp.supported);
}

#[tokio::test]
async fn vapix_error_envelope_maps_to_unsupported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(STREAM_PROFILES))
        .respond_with(json_reply(fixture!("error_unsupported_method.json")))
        .mount(&server)
        .await;
    let err = provider_for(&server, creds(PASS))
        .get_stream_info()
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Unsupported);
}

#[tokio::test]
async fn malformed_json_is_a_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(BASIC_DEVICE_INFO))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;
    let err = provider_for(&server, None).probe().await.unwrap_err();
    assert_eq!(err.category(), ErrorCategory::ProtocolError);
}

#[tokio::test]
async fn slow_camera_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(BASIC_DEVICE_INFO))
        .respond_with(
            json_reply(fixture!("basicdeviceinfo_unrestricted.json"))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;
    let err = provider_for(&server, None).probe().await.unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Timeout);
}

#[tokio::test]
async fn closed_port_is_unreachable() {
    let p = AxisProvider::new(
        &ProviderTarget::simple("127.0.0.1", 1, false),
        Duration::from_secs(2),
    );
    assert_eq!(
        p.probe().await.unwrap_err().category(),
        ErrorCategory::Unreachable
    );
}

#[tokio::test]
async fn connect_timeout_is_unreachable_not_timeout() {
    // RFC 5737 TEST-NET-1 never answers: depending on the network this either
    // fails immediately or hits the connect timeout. Both mean "unreachable".
    let p = AxisProvider::new(
        &ProviderTarget::simple("192.0.2.1", 80, false),
        Duration::from_secs(1),
    );
    assert_eq!(
        p.probe().await.unwrap_err().category(),
        ErrorCategory::Unreachable
    );
}

#[tokio::test]
async fn axis_provider_passes_contract_suite() {
    let healthy = MockServer::start().await;
    mount_camera(&healthy, true).await;
    let auth_failure = MockServer::start().await;
    mount_camera(&auth_failure, true).await;
    // Live video for the healthy camera: a fake RTSP server with the same
    // account and the profiles from `streamprofile_list.json`.
    let rtsp = super::rtsp_tests::FakeCamera::start_with(super::rtsp_tests::FakeConfig {
        user: USER,
        password: PASS,
        profiles: vec![
            "My full HD profile",
            "My inverted image profile",
            "My high compression profile",
        ],
    })
    .await;
    contract::run(contract::Subjects {
        healthy: Arc::new(provider_for(&healthy, creds(PASS)).with_rtsp_port(rtsp.port)),
        unreachable: Arc::new(AxisProvider::new(
            &ProviderTarget::simple("127.0.0.1", 1, false),
            Duration::from_secs(2),
        )),
        auth_failure: Arc::new(provider_for(&auth_failure, creds("wrong-password"))),
    })
    .await;
}

// ---------------------------------------------------------------------------
// TLS modes against a real HTTPS server with a self-signed certificate
// ---------------------------------------------------------------------------

struct TlsCamera {
    port: u16,
    fingerprint: String,
    _shutdown: tokio_util::sync::CancellationToken,
}

async fn tls_camera() -> TlsCamera {
    use axum::routing::post;
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let fingerprint = crate::tls::fingerprint(cert.cert.der());
    let config = axum_server::tls_rustls::RustlsConfig::from_der(
        vec![cert.cert.der().to_vec()],
        cert.signing_key.serialize_der(),
    )
    .await
    .unwrap();
    let app = axum::Router::new().route(
        SYSTEM_READY,
        post(|| async {
            (
                [("content-type", "application/json")],
                fixture!("systemready.json"),
            )
        }),
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = axum_server::Handle::new();
    let shutdown = tokio_util::sync::CancellationToken::new();
    {
        let handle = handle.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            shutdown.cancelled().await;
            handle.shutdown();
        });
    }
    tokio::spawn(
        axum_server::from_tcp_rustls(listener, config)
            .unwrap()
            .handle(handle)
            .serve(app.into_make_service()),
    );
    TlsCamera {
        port,
        fingerprint,
        _shutdown: shutdown,
    }
}

impl Drop for TlsCamera {
    fn drop(&mut self) {
        self._shutdown.cancel();
    }
}

fn https_provider(port: u16, tls: TlsPolicy) -> AxisProvider {
    AxisProvider::new(
        &ProviderTarget {
            host: "127.0.0.1".into(),
            port,
            https: true,
            tls,
            credentials: None,
            allow_basic_over_http: false,
        },
        Duration::from_secs(3),
    )
}

#[tokio::test]
async fn tls_modes() {
    let cam = tls_camera().await;

    // Strict: a self-signed certificate is not trusted.
    let strict = https_provider(cam.port, TlsPolicy::Strict);
    let err = strict.get_status().await.unwrap_err();
    assert_eq!(err.category(), ErrorCategory::TlsError, "{err}");
    assert_eq!(
        strict.tls_fingerprint().as_deref(),
        Some(cam.fingerprint.as_str())
    );

    // Pinned to the right fingerprint: trusted.
    let pinned = https_provider(cam.port, TlsPolicy::Pinned(cam.fingerprint.clone()));
    assert_eq!(
        pinned.get_status().await.unwrap().uptime_seconds,
        Some(7800)
    );

    // Pinned to a different fingerprint: rejected.
    let wrong = https_provider(cam.port, TlsPolicy::Pinned("AA:".repeat(31) + "AA"));
    assert_eq!(
        wrong.get_status().await.unwrap_err().category(),
        ErrorCategory::TlsError
    );

    // Insecure: accepted, and the fingerprint is still recorded.
    let insecure = https_provider(cam.port, TlsPolicy::InsecureSkipVerify);
    assert!(insecure.get_status().await.is_ok());
    assert_eq!(
        insecure.tls_fingerprint().as_deref(),
        Some(cam.fingerprint.as_str())
    );
}

#[test]
fn prefix_masks_and_versions() {
    assert_eq!(prefix_to_mask(24).as_deref(), Some("255.255.255.0"));
    assert_eq!(prefix_to_mask(0).as_deref(), Some("0.0.0.0"));
    assert_eq!(prefix_to_mask(33), None);
    assert!(version_gt("1.10", "1.9"));
    assert!(!version_gt("1.0", "2.0"));
}
