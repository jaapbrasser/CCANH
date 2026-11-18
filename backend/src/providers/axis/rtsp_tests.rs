//! AxisProvider live video against an in-process fake AXIS RTSP server:
//! Digest auth, DESCRIBE/SETUP/PLAY, and H.264 RTP (single NAL + FU-A)
//! interleaved on the RTSP connection, built from a bundled demo clip. This
//! runs the real `retina` client end to end, without a camera or MediaMTX.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use md5::{Digest as _, Md5};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::tls::TlsPolicy;
use super::*;
use crate::credentials::Credentials;
use crate::secret::SecretString;
use crate::streaming::h264;
use crate::streaming::media::MediaEvent;

const USER: &str = "viewer";
const PASS: &str = "rtsp-test-password";
const REALM: &str = "AXIS_ACCC8E000011";
const NONCE: &str = "0000f00dcafe";
const SSRC: u32 = 0x1234_5678;
const FPS: u32 = 15;
const FRAMES: usize = 40;

const SDP: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/vapix/rtsp_describe_h264.sdp"
));
const CLIP: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../assets/demo/testsrc2-grid.h264"
));

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Value of `key="..."` (or `key=...`) in a Digest header.
fn param<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    header.split(',').find_map(|part| {
        let (k, v) = part.trim().trim_start_matches("Digest ").split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"'))
    })
}

fn authorized(cfg: &FakeConfig, method: &str, auth: Option<&str>) -> bool {
    let Some(h) = auth.filter(|h| h.starts_with("Digest ")) else {
        return false;
    };
    let (Some(user), Some(uri), Some(response)) =
        (param(h, "username"), param(h, "uri"), param(h, "response"))
    else {
        return false;
    };
    let ha1 = md5_hex(&format!("{user}:{REALM}:{}", cfg.password));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    user == cfg.user && response == md5_hex(&format!("{ha1}:{NONCE}:{ha2}"))
}

/// One RTP packet carrying `payload`.
fn rtp(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + payload.len());
    p.push(0x80);
    p.push(96 | if marker { 0x80 } else { 0 });
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&SSRC.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

/// RFC 6184 packetization of one access unit (AVC, 4-byte lengths).
fn packetize(avc: &[u8], seq: &mut u16, ts: u32) -> Vec<Vec<u8>> {
    const MTU: usize = 1400;
    let mut nals = Vec::new();
    let mut rest = avc;
    while rest.len() >= 4 {
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        nals.push(&rest[4..4 + len]);
        rest = &rest[4 + len..];
    }
    let mut out = Vec::new();
    for (i, nal) in nals.iter().enumerate() {
        let last_nal = i + 1 == nals.len();
        if nal.len() <= MTU {
            out.push(rtp(*seq, ts, last_nal, nal));
            *seq = seq.wrapping_add(1);
            continue;
        }
        let indicator = (nal[0] & 0xE0) | 28;
        let chunks: Vec<&[u8]> = nal[1..].chunks(MTU).collect();
        for (j, chunk) in chunks.iter().enumerate() {
            let start = if j == 0 { 0x80 } else { 0 };
            let end = if j + 1 == chunks.len() { 0x40 } else { 0 };
            let mut payload = vec![indicator, start | end | (nal[0] & 0x1F)];
            payload.extend_from_slice(chunk);
            out.push(rtp(*seq, ts, last_nal && end != 0, &payload));
            *seq = seq.wrapping_add(1);
        }
    }
    out
}

/// Who the fake camera accepts and which stream profiles it has (the default
/// stream, `streamprofile` omitted, always works).
pub(super) struct FakeConfig {
    pub user: &'static str,
    pub password: &'static str,
    pub profiles: Vec<&'static str>,
}

/// A fake camera's RTSP server. Records every request it receives (headers
/// included) so tests can check what was sent.
pub(super) struct FakeCamera {
    pub port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeCamera {
    async fn start() -> Self {
        Self::start_with(FakeConfig {
            user: USER,
            password: PASS,
            profiles: vec!["grid"],
        })
        .await
    }

    pub async fn start_with(cfg: FakeConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        let cfg = Arc::new(cfg);
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(serve(sock, Arc::clone(&cfg), Arc::clone(&log)));
            }
        });
        Self { port, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

async fn send(w: &Arc<tokio::sync::Mutex<OwnedWriteHalf>>, bytes: &[u8]) -> bool {
    w.lock().await.write_all(bytes).await.is_ok()
}

async fn serve(sock: tokio::net::TcpStream, cfg: Arc<FakeConfig>, log: Arc<Mutex<Vec<String>>>) {
    let es = h264::parse_annexb(CLIP).unwrap();
    let (r, w) = sock.into_split();
    let w = Arc::new(tokio::sync::Mutex::new(w));
    let mut r = BufReader::new(r);
    loop {
        let mut head = Vec::new();
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end().to_owned();
            if line.is_empty() {
                break;
            }
            head.push(line);
        }
        log.lock().unwrap().push(head.join("\n"));
        let mut first = head[0].split(' ');
        let (method, uri) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
        let header = |name: &str| {
            head.iter().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
            })
        };
        let cseq = header("CSeq").unwrap_or_default();
        let reply = |status: &str, extra: &str, body: &str| {
            let mut s = format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\n{extra}");
            if !body.is_empty() {
                s.push_str(&format!("Content-Length: {}\r\n", body.len()));
            }
            s.push_str("\r\n");
            s.push_str(body);
            s
        };

        if method != "OPTIONS" && !authorized(&cfg, method, header("Authorization").as_deref()) {
            let challenge =
                format!("WWW-Authenticate: Digest realm=\"{REALM}\", nonce=\"{NONCE}\"\r\n");
            if !send(&w, reply("401 Unauthorized", &challenge, "").as_bytes()).await {
                return;
            }
            continue;
        }
        let base = uri.trim_end_matches('/').to_owned();
        let response = match method {
            "OPTIONS" => reply(
                "200 OK",
                "Public: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN, GET_PARAMETER\r\n",
                "",
            ),
            "DESCRIBE" if !known_profile(&cfg, uri) => reply("404 Not Found", "", ""),
            "DESCRIBE" => {
                let b64 = base64::engine::general_purpose::STANDARD;
                let sdp = SDP
                    .replace("{host}", "127.0.0.1")
                    .replace("{base}", &base)
                    .replace("{plid}", &hex(&es.sps[1..4]))
                    .replace(
                        "{sprop}",
                        &format!("{},{}", b64.encode(&es.sps), b64.encode(&es.pps)),
                    )
                    .replace('\n', "\r\n");
                reply(
                    "200 OK",
                    &format!("Content-Type: application/sdp\r\nContent-Base: {base}/\r\n"),
                    &sdp,
                )
            }
            "SETUP" => reply(
                "200 OK",
                &format!(
                    "Transport: RTP/AVP/TCP;unicast;interleaved=0-1;ssrc={SSRC:08X};mode=\"PLAY\"\r\nSession: 1234ABCD;timeout=60\r\n"
                ),
                "",
            ),
            "PLAY" => {
                let extra = format!(
                    "Session: 1234ABCD\r\nRange: npt=now-\r\nRTP-Info: url={base}/trackID=1;seq=1;rtptime=0\r\n"
                );
                if !send(&w, reply("200 OK", &extra, "").as_bytes()).await {
                    return;
                }
                let units: Vec<_> = es.units.iter().map(|u| u.avc.clone()).collect();
                let w = Arc::clone(&w);
                tokio::spawn(async move {
                    let mut seq: u16 = 1;
                    for i in 0..FRAMES {
                        let ts = u32::try_from(i).unwrap() * (90_000 / FPS);
                        for pkt in packetize(&units[i % units.len()], &mut seq, ts) {
                            let mut frame = vec![b'$', 0];
                            frame.extend_from_slice(&u16::try_from(pkt.len()).unwrap().to_be_bytes());
                            frame.extend_from_slice(&pkt);
                            if !send(&w, &frame).await {
                                return;
                            }
                        }
                    }
                });
                continue;
            }
            _ => reply("200 OK", "Session: 1234ABCD\r\n", ""),
        };
        if !send(&w, response.as_bytes()).await {
            return;
        }
    }
}

/// Whether the `streamprofile` in a request URI (if any) is one the camera has.
fn known_profile(cfg: &FakeConfig, uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    url.query_pairs()
        .find(|(k, _)| k == "streamprofile")
        .is_none_or(|(_, v)| cfg.profiles.iter().any(|p| *p == v))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn provider(port: u16, password: &str) -> AxisProvider {
    AxisProvider::new(
        &ProviderTarget {
            host: "127.0.0.1".into(),
            port: 80,
            https: false,
            tls: TlsPolicy::Strict,
            credentials: Some(Credentials {
                username: USER.into(),
                password: SecretString::from(password.to_owned()),
            }),
            allow_basic_over_http: false,
        },
        Duration::from_secs(5),
    )
    .with_rtsp_port(port)
}

#[tokio::test]
async fn plays_h264_from_the_camera_with_digest_auth() {
    let cam = FakeCamera::start().await;
    let mut src = provider(cam.port, PASS)
        .open_stream(&StreamProfileId("grid".into()), CancellationToken::new())
        .await
        .unwrap();

    let Some(MediaEvent::Params(p)) = src.next().await.unwrap() else {
        panic!("parameters come first")
    };
    assert!(p.rfc6381.starts_with("avc1.4D40"), "{}", p.rfc6381);
    assert_eq!((p.width, p.height), (640, 360));
    assert_eq!(&p.sample_entry[4..8], b"avc1");

    let mut dts = Vec::new();
    let mut keyframes = Vec::new();
    while dts.len() < 20 {
        match tokio::time::timeout(Duration::from_secs(5), src.next())
            .await
            .unwrap()
            .unwrap()
        {
            Some(MediaEvent::Frame(f)) => {
                assert_eq!(f.pts, f.dts);
                assert!(f.data.len() > 4);
                keyframes.push(f.keyframe);
                dts.push(f.dts);
            }
            Some(MediaEvent::Params(_)) => panic!("parameters didn't change"),
            None => panic!("stream ended early"),
        }
    }
    assert!(keyframes[0], "the clip starts with a keyframe");
    assert!(
        dts.windows(2)
            .all(|w| w[1] - w[0] == i64::from(90_000 / FPS)),
        "{dts:?}"
    );

    let requests = cam.requests();
    let describe = requests
        .iter()
        .find(|r| r.starts_with("DESCRIBE") && r.contains("Authorization"))
        .expect("an authorized DESCRIBE");
    assert!(
        describe.starts_with(&format!(
            "DESCRIBE rtsp://127.0.0.1:{}/axis-media/media.amp?videocodec=h264&audio=0&streamprofile=grid RTSP/1.0",
            cam.port
        )),
        "{describe}"
    );
    assert!(
        requests
            .iter()
            .all(|r| !r.contains(PASS) && !r.contains("Basic ")),
        "the password is never sent in clear"
    );
    assert!(requests.iter().any(|r| r.starts_with("PLAY")));
}

#[tokio::test]
async fn default_profile_omits_streamprofile() {
    let p = provider(554, PASS);
    let url = p.rtsp_url(&StreamProfileId(String::new())).unwrap();
    assert_eq!(
        url.as_str(),
        "rtsp://127.0.0.1:554/axis-media/media.amp?videocodec=h264&audio=0"
    );
    let named = p.rtsp_url(&StreamProfileId("My profile&1".into())).unwrap();
    assert!(
        named.as_str().ends_with("&streamprofile=My+profile%261"),
        "{named}"
    );
    assert!(named.password().is_none() && named.username().is_empty());
}

#[tokio::test]
async fn rejected_credentials_are_auth_failed() {
    let cam = FakeCamera::start().await;
    let err = provider(cam.port, "wrong")
        .open_stream(&StreamProfileId("grid".into()), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(err.category(), ErrorCategory::AuthFailed, "{err}");
}

#[tokio::test]
async fn unknown_profile_is_unsupported() {
    let cam = FakeCamera::start().await;
    let err = provider(cam.port, PASS)
        .open_stream(&StreamProfileId("missing".into()), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(err.category(), ErrorCategory::Unsupported, "{err}");
}

#[tokio::test]
async fn closed_port_is_unreachable() {
    // Bind and drop to get a port with nothing listening.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = provider(port, PASS)
        .open_stream(&StreamProfileId(String::new()), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert_eq!(err.category(), ErrorCategory::Unreachable, "{err}");
}
