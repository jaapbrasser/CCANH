//! Streaming spike (SPEC §7.3, option A): one RTSP upstream per profile, shared
//! by every viewer, remuxed (no transcoding) to fragmented MP4 and pushed over
//! WebSocket to Media Source Extensions in the browser.
//!
//! Throwaway code: correctness shortcuts are deliberate. See ADR 0002.
//!
//!   cargo run --release -- rtsp://127.0.0.1:8554/   (profiles at test/{profile})
//!   open http://127.0.0.1:9000/?profile=main

mod fmp4;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use futures::StreamExt;
use retina::client::{PlayOptions, Session, SessionOptions, SetupOptions};
use retina::codec::{CodecItem, ParametersRef};
use tokio::sync::{broadcast, watch};

const GOP_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Clone)]
struct Init {
    codec: String,
    segment: Bytes,
}

struct Frag {
    seq: u64,
    key: bool,
    /// When the frame arrived from upstream (wall clock, ms).
    ingest_ms: u64,
    /// Decode timestamp relative to stream start (ms).
    pts_ms: u64,
    /// moof + mdat
    bytes: Bytes,
}

#[derive(Default)]
struct Stats {
    upstream_sessions: AtomicU64,
    upstream_bytes: AtomicU64,
    frames: AtomicU64,
    sent_bytes: AtomicU64,
    subscribers: AtomicUsize,
    lag_resyncs: AtomicU64,
}

struct Hub {
    init: watch::Sender<Option<Init>>,
    frames: broadcast::Sender<Arc<Frag>>,
    gop: Mutex<Vec<Arc<Frag>>>,
    stats: Stats,
}

impl Hub {
    fn start(url: url::Url) -> Arc<Hub> {
        let (init, _) = watch::channel(None);
        let (frames, _) = broadcast::channel(256);
        let hub = Arc::new(Hub {
            init,
            frames,
            gop: Mutex::new(Vec::new()),
            stats: Stats::default(),
        });
        let h = hub.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = h.run_upstream(url.clone()).await {
                    tracing::warn!("upstream {url}: {e:#}; reconnecting in 1s");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        hub
    }

    async fn run_upstream(&self, url: url::Url) -> anyhow::Result<()> {
        let mut session = Session::describe(url, SessionOptions::default()).await?;
        let video = session
            .streams()
            .iter()
            .position(|s| s.media() == "video" && s.encoding_name() == "h264")
            .ok_or_else(|| anyhow::anyhow!("no H.264 video stream"))?;
        session.setup(video, SetupOptions::default()).await?;
        let mut demuxed = session.play(PlayOptions::default()).await?.demuxed()?;
        self.stats.upstream_sessions.fetch_add(1, Ordering::Relaxed);
        tracing::info!("upstream session opened");

        let mut seq: u64 = 0;
        let mut last_ts: Option<i64> = None;
        let mut duration: u32 = 3600;
        let mut have_init = false;

        while let Some(item) = demuxed.next().await {
            let CodecItem::VideoFrame(frame) = item? else { continue };
            if !have_init || frame.has_new_parameters() {
                if let Some(ParametersRef::Video(p)) = demuxed.streams()[video].parameters() {
                    let (w, h) = p.pixel_dimensions();
                    let entry = p.mp4_sample_entry().build()?;
                    self.init.send_replace(Some(Init {
                        codec: p.rfc6381_codec().to_owned(),
                        segment: fmp4::init_segment(w, h, &entry).into(),
                    }));
                    have_init = true;
                }
            }
            if !have_init {
                continue;
            }
            let ts = frame.timestamp().elapsed();
            if let Some(prev) = last_ts {
                duration = u32::try_from(ts - prev).unwrap_or(3600).max(1);
            }
            last_ts = Some(ts);
            let key = frame.is_random_access_point();
            let data = frame.data();
            self.stats.upstream_bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
            self.stats.frames.fetch_add(1, Ordering::Relaxed);
            seq += 1;
            let frag = Arc::new(Frag {
                seq,
                key,
                ingest_ms: now_ms(),
                pts_ms: (ts.max(0) as u64) * 1000 / u64::from(fmp4::TIMESCALE),
                bytes: fmp4::fragment(seq as u32, ts.max(0) as u64, duration, key, data).into(),
            });
            {
                let mut gop = self.gop.lock().unwrap();
                if key {
                    gop.clear();
                }
                let size: usize = gop.iter().map(|f| f.bytes.len()).sum();
                if size + frag.bytes.len() <= GOP_CACHE_MAX_BYTES {
                    gop.push(frag.clone());
                }
            }
            let _ = self.frames.send(frag);
        }
        Ok(())
    }
}

#[derive(Clone)]
struct App {
    base: url::Url,
    hubs: Arc<Mutex<HashMap<String, Arc<Hub>>>>,
}

impl App {
    /// Get or start the hub for a profile. The lock makes "check, then start"
    /// atomic, so concurrent viewers never open a second upstream connection.
    fn hub(&self, profile: &str) -> Arc<Hub> {
        let mut hubs = self.hubs.lock().unwrap();
        hubs.entry(profile.to_owned())
            .or_insert_with(|| Hub::start(self.base.join(&format!("test/{profile}")).unwrap()))
            .clone()
    }
}

fn framed(f: &Frag) -> Bytes {
    let mut out = Vec::with_capacity(16 + f.bytes.len());
    out.extend_from_slice(&f.ingest_ms.to_be_bytes());
    out.extend_from_slice(&f.pts_ms.to_be_bytes());
    out.extend_from_slice(&f.bytes);
    out.into()
}

async fn ws_handler(
    Path(profile): Path<String>,
    State(app): State<App>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let hub = app.hub(&profile);
    ws.on_upgrade(move |socket| viewer(socket, hub))
}

async fn viewer(mut socket: WebSocket, hub: Arc<Hub>) {
    hub.stats.subscribers.fetch_add(1, Ordering::Relaxed);
    let _guard = scopeguard(&hub);

    let mut init_rx = hub.init.subscribe();
    let init = loop {
        if let Some(init) = init_rx.borrow().clone() {
            break init;
        }
        if init_rx.changed().await.is_err() {
            return;
        }
    };
    let hello = serde_json::json!({ "type": "init", "codec": init.codec }).to_string();
    if socket.send(Message::Text(hello.into())).await.is_err()
        || socket.send(Message::Binary(init.segment.clone())).await.is_err()
    {
        return;
    }

    // Subscribe first, then replay the cached GOP, then skip live frames the
    // replay already covered: no gap and no duplicates.
    let mut rx = hub.frames.subscribe();
    let cached: Vec<Arc<Frag>> = hub.gop.lock().unwrap().clone();
    let mut last_seq = 0;
    for f in &cached {
        let b = framed(f);
        hub.stats.sent_bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
        if socket.send(Message::Binary(b)).await.is_err() {
            return;
        }
        last_seq = f.seq;
    }
    let mut waiting_for_key = cached.is_empty();

    loop {
        tokio::select! {
            msg = socket.recv() => match msg {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                _ => {}
            },
            f = rx.recv() => match f {
                Ok(f) => {
                    if f.seq <= last_seq { continue; }
                    if waiting_for_key && !f.key { continue; }
                    waiting_for_key = false;
                    let b = framed(&f);
                    hub.stats.sent_bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                    if socket.send(Message::Binary(b)).await.is_err() { return; }
                }
                // Slow viewer: never send a partial GOP; resync at the next keyframe.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    hub.stats.lag_resyncs.fetch_add(1, Ordering::Relaxed);
                    waiting_for_key = true;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

struct SubGuard<'a>(&'a Hub);
impl Drop for SubGuard<'_> {
    fn drop(&mut self) {
        self.0.stats.subscribers.fetch_sub(1, Ordering::Relaxed);
    }
}
fn scopeguard(hub: &Hub) -> SubGuard<'_> {
    SubGuard(hub)
}

async fn stats(State(app): State<App>) -> Json<serde_json::Value> {
    let hubs = app.hubs.lock().unwrap();
    let map: serde_json::Map<String, serde_json::Value> = hubs
        .iter()
        .map(|(k, h)| {
            let s = &h.stats;
            (
                k.clone(),
                serde_json::json!({
                    "upstream_sessions": s.upstream_sessions.load(Ordering::Relaxed),
                    "upstream_bytes": s.upstream_bytes.load(Ordering::Relaxed),
                    "frames": s.frames.load(Ordering::Relaxed),
                    "sent_bytes": s.sent_bytes.load(Ordering::Relaxed),
                    "subscribers": s.subscribers.load(Ordering::Relaxed),
                    "lag_resyncs": s.lag_resyncs.load(Ordering::Relaxed),
                    "gop_cache_frames": h.gop.lock().unwrap().len(),
                }),
            )
        })
        .collect();
    Json(serde_json::json!({ "now_ms": now_ms(), "hubs": map }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info,retina=warn").init();
    let base = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rtsp://127.0.0.1:8554/".into());
    let base = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))?;
    let app = App {
        base,
        hubs: Arc::default(),
    };
    let router = Router::new()
        .route("/", get(|| async { Html(include_str!("page.html")) }))
        .route("/ws/{profile}", get(ws_handler))
        .route("/stats", get(stats))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9000").await?;
    tracing::info!("spike on http://127.0.0.1:9000/?profile=main");
    axum::serve(listener, router).await?;
    Ok(())
}
