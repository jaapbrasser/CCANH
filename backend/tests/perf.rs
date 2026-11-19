//! Load test at the SPEC §18 Q3 target scale, in demo mode: 250 cameras
//! polled every 10 s, 25 signed-in users (each with a live-updates socket and
//! an API request every second), and 50 camera streams with 2 viewers each.
//!
//! Ignored by default. Run it with a release build:
//!
//! ```text
//! cargo test --release -p crane --test perf -- --ignored --nocapture
//! ```
//!
//! `CRANE_PERF_SECS` sets the measured duration (default 60). Results are
//! recorded in docs/performance.md.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crane::auth::{password, Role};
use crane::cameras::{self, CameraSettings, Scheme, TlsMode};
use crane::config::{Config, Mode};
use crane::secret::SecretString;
use crane::{storage, users, AppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

const USERS: usize = 25;
const STREAM_CAMERAS: usize = 25; // × (main + grid) = 50 upstreams
const EXTRA_CAMERAS: u8 = 226; // + 24 demo cameras = 250
const PASSWORD: &str = "load-test-password-1234";

struct Server {
    base: String,
    origin: String,
    http: reqwest::Client,
}

impl Server {
    async fn login(&self, username: &str, password: &str) -> String {
        let res = self
            .http
            .post(format!("{}/api/v1/auth/login", self.base))
            .header("origin", &self.origin)
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()
            .await
            .unwrap();
        assert!(
            res.status().is_success(),
            "login {username}: {}",
            res.status()
        );
        let cookie = res
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("crane_session="))
            .unwrap();
        cookie.split(';').next().unwrap().to_owned()
    }

    async fn get(&self, cookie: &str, path: &str) -> Value {
        self.http
            .get(format!("{}{path}", self.base))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn ws(
        &self,
        cookie: &str,
        path: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut req = format!("{}{path}", self.base.replacen("http", "ws", 1))
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("cookie", HeaderValue::from_str(cookie).unwrap());
        req.headers_mut()
            .insert("origin", HeaderValue::from_str(&self.origin).unwrap());
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

/// Resident memory (MiB) and CPU seconds of this process, where `ps` exists.
fn process_usage() -> Option<(f64, String)> {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=,time=", "-p", &pid])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    let rss_kb: f64 = parts.next()?.parse().ok()?;
    Some((rss_kb / 1024.0, parts.next()?.to_owned()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test; run with --release -- --ignored --nocapture"]
async fn target_scale() {
    let secs: u64 = std::env::var("CRANE_PERF_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        mode: Mode::Demo,
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let db = storage::connect(&config.database_path()).await.unwrap();
    let store: Arc<dyn crane::credentials::CredentialStore> =
        Arc::new(crane::credentials::AesGcmStore::ephemeral());
    crane::demo::seed(&db, &*store).await.unwrap();
    for i in 1..=EXTRA_CAMERAS {
        let s = CameraSettings {
            name: format!("Load {i:03}"),
            // RFC 5737 TEST-NET-3: never routed; the demo provider simulates them.
            host: format!("203.0.113.{i}"),
            port: 443,
            scheme: Scheme::Https,
            tls_mode: TlsMode::Strict,
            tls_pinned_sha256: None,
            monitoring_enabled: true,
            allow_basic_over_http: false,
        };
        cameras::create(&db, &*store, &s, None).await.unwrap();
    }
    let hash = password::hash(&SecretString::from(PASSWORD.to_owned())).unwrap();
    for u in 1..=USERS {
        users::create(&db, &format!("load{u:02}"), &hash, Role::Viewer)
            .await
            .unwrap();
    }
    let state = AppState::new(config, db, store);
    state.settings.load(&state.db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let stats = state.monitor_stats.clone();
    tokio::spawn(crane::serve(
        listener,
        state.clone(),
        None,
        shutdown.clone(),
    ));

    let server = Arc::new(Server {
        base: format!("http://{addr}"),
        origin: format!("http://{addr}"),
        // Plain HTTP only; reqwest still needs a provider to build.
        http:
            reqwest::Client::builder()
                .use_preconfigured_tls(
                    rustls::ClientConfig::builder_with_provider(
                        crane::providers::axis::tls::provider(),
                    )
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_root_certificates(rustls::RootCertStore::empty())
                    .with_no_client_auth(),
                )
                .build()
                .unwrap(),
    });
    let admin = server.login("admin", "crane-demo-admin").await;
    let mut cookies = Vec::new();
    for u in 1..=USERS {
        cookies.push(server.login(&format!("load{u:02}"), PASSWORD).await);
    }

    // Stream cameras: ones whose identity (and H.264 profiles) has loaded.
    let started = Instant::now();
    let stream_cameras: Vec<String> = loop {
        let list = server
            .get(&admin, "/api/v1/cameras?q=Load&page_size=200")
            .await;
        let mut ready = Vec::new();
        for c in list["items"].as_array().unwrap() {
            let id = c["id"].as_str().unwrap();
            let d = server.get(&admin, &format!("/api/v1/cameras/{id}")).await;
            let h264 = d["stream_profiles"].as_array().is_some_and(|p| {
                p.iter()
                    .filter(|p| p["codec"] == "h264" || p["codec"].is_null())
                    .count()
                    >= 2
            });
            if h264 {
                ready.push(id.to_owned());
            }
            if ready.len() == STREAM_CAMERAS {
                break;
            }
        }
        if ready.len() == STREAM_CAMERAS {
            break ready;
        }
        assert!(
            started.elapsed() < Duration::from_secs(90),
            "only {} cameras ready",
            ready.len()
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    println!(
        "setup: {} cameras ready to stream after {:?}",
        stream_cameras.len(),
        started.elapsed()
    );

    let deadline = Instant::now() + Duration::from_secs(secs);
    let checks_before = stats.snapshot().checks_total;
    let live_messages = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();

    // 25 users: a live-updates socket each.
    for cookie in &cookies {
        let mut ws = server.ws(cookie, "/api/v1/ws").await;
        ws.send(Message::Text(
            r#"{"type":"subscribe","topics":["camera_status","cameras","tasks"]}"#.into(),
        ))
        .await
        .unwrap();
        let count = live_messages.clone();
        tasks.push(tokio::spawn(async move {
            while Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_secs(1), ws.next()).await {
                    Ok(Some(Ok(Message::Text(_)))) => {
                        count.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Some(Ok(_))) | Err(_) => {}
                    Ok(None) | Ok(Some(Err(_))) => break,
                }
            }
            (0u64, 0u64, false)
        }));
    }

    // 50 streams (main + grid per camera), 2 viewers each = 100 media sockets.
    let mut media = Vec::new();
    for (j, cam) in stream_cameras.iter().enumerate() {
        for (purpose, fps) in [("detail", 25u64), ("grid", 15u64)] {
            for v in 0..2 {
                let cookie = &cookies[(j * 4 + v + if purpose == "grid" { 2 } else { 0 }) % USERS];
                let mut ws = server
                    .ws(
                        cookie,
                        &format!("/api/v1/streams/{cam}/ws?purpose={purpose}"),
                    )
                    .await;
                media.push(tokio::spawn(async move {
                    let mut frames = 0u64;
                    let mut first_keyframe = None;
                    let mut init = false;
                    while Instant::now() < deadline {
                        match tokio::time::timeout(Duration::from_secs(1), ws.next()).await {
                            Ok(Some(Ok(Message::Binary(b)))) if init => {
                                frames += 1;
                                if first_keyframe.is_none() {
                                    first_keyframe = Some(b[0] & 1 == 1);
                                }
                            }
                            Ok(Some(Ok(Message::Binary(_)))) => init = true,
                            Ok(Some(Ok(_))) | Err(_) => {}
                            Ok(None) | Ok(Some(Err(_))) => break,
                        }
                    }
                    (frames, fps, first_keyframe.unwrap_or(false))
                }));
            }
        }
    }

    // Each user requests a page of cameras every second.
    let latencies = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    for cookie in cookies.clone() {
        let (server, latencies) = (server.clone(), latencies.clone());
        tasks.push(tokio::spawn(async move {
            while Instant::now() < deadline {
                let t = Instant::now();
                let res = server
                    .http
                    .get(format!("{}/api/v1/cameras?page_size=50", server.base))
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .unwrap();
                assert!(res.status().is_success());
                let _ = res.bytes().await;
                latencies.lock().await.push(t.elapsed().as_micros());
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            (0, 0, false)
        }));
    }

    // Stream metrics while everyone is still watching.
    let snapshot = {
        let (server, admin) = (server.clone(), admin.clone());
        tokio::spawn(async move {
            tokio::time::sleep_until((deadline - Duration::from_secs(2)).into()).await;
            server.get(&admin, "/api/v1/streams").await
        })
    };
    let mut frame_ratios = Vec::new();
    let mut keyframe_starts = 0;
    for m in media {
        let (frames, fps, key) = m.await.unwrap();
        // Allow the first couple of seconds for startup.
        frame_ratios.push(frames as f64 / (fps * secs.saturating_sub(2)) as f64);
        keyframe_starts += usize::from(key);
    }
    for t in tasks {
        t.await.unwrap();
    }
    let streams = snapshot.await.unwrap();
    let streams = streams.as_array().unwrap();
    let lag_resyncs: u64 = streams
        .iter()
        .map(|s| s["lag_resyncs"].as_u64().unwrap_or(0))
        .sum();
    let reconnects: u64 = streams
        .iter()
        .map(|s| s["reconnects"].as_u64().unwrap_or(0))
        .sum();
    let out_mbps: f64 = streams
        .iter()
        .map(|s| s["out_bytes_per_sec"].as_f64().unwrap_or(0.0))
        .sum::<f64>()
        * 8.0
        / 1e6;
    let in_mbps: f64 = streams
        .iter()
        .map(|s| s["in_bytes_per_sec"].as_f64().unwrap_or(0.0))
        .sum::<f64>()
        * 8.0
        / 1e6;
    let checks = stats.snapshot().checks_total - checks_before;
    let mut lat = latencies.lock().await.clone();
    lat.sort_unstable();
    frame_ratios.sort_by(f64::total_cmp);
    let usage = process_usage();
    shutdown.cancel();

    println!("\n=== CCANH load test: {secs} s at target scale (demo mode) ===");
    println!(
        "cameras: 250 (status checks: {checks}, {:.1}/s)",
        checks as f64 / secs as f64
    );
    println!(
        "users: {USERS} live sockets ({} live messages received)",
        live_messages.load(Ordering::Relaxed)
    );
    println!(
        "streams: {} upstreams, {} viewers; inbound {in_mbps:.1} Mbit/s, outbound {out_mbps:.1} Mbit/s",
        streams.len(),
        streams.iter().map(|s| s["subscribers"].as_u64().unwrap_or(0)).sum::<u64>()
    );
    println!(
        "frames delivered / expected: min {:.2}, median {:.2}; first fragment a keyframe: {keyframe_starts}/{}",
        frame_ratios[0],
        frame_ratios[frame_ratios.len() / 2],
        frame_ratios.len()
    );
    println!("lag resyncs: {lag_resyncs}, reconnects: {reconnects}");
    println!(
        "GET /api/v1/cameras?page_size=50 ({} requests): p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        lat.len(),
        percentile(&lat, 0.5) as f64 / 1000.0,
        percentile(&lat, 0.95) as f64 / 1000.0,
        lat.last().copied().unwrap_or(0) as f64 / 1000.0
    );
    if let Some((rss, cpu)) = usage {
        println!("process (server + load clients): RSS {rss:.0} MiB, CPU time {cpu}");
    }

    assert_eq!(
        streams.len(),
        2 * STREAM_CAMERAS,
        "one upstream per camera and profile"
    );
    assert!(
        streams.iter().all(|s| s["subscribers"] == 2),
        "two viewers on every stream"
    );
    assert_eq!(
        keyframe_starts,
        frame_ratios.len(),
        "every viewer starts at a keyframe"
    );
    assert!(
        frame_ratios[0] > 0.8,
        "a viewer fell behind: {:.2}",
        frame_ratios[0]
    );
    assert_eq!(lag_resyncs, 0, "no viewer fell behind its queue");
    assert!(percentile(&lat, 0.95) < 250_000, "API p95 under 250 ms");
    // 250 cameras every 10 s = 25/s; offline ones back off, so allow slack.
    assert!(
        checks as f64 / secs as f64 > 15.0,
        "monitoring kept up: {checks} checks"
    );
}
