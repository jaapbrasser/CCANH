//! The stream manager (SPEC §7.2): one upstream connection per (camera,
//! profile), shared by every viewer.
//!
//! - Subscribing to a running stream starts the viewer at the most recent
//!   keyframe from the GOP cache. Subscribing while it is starting joins the
//!   same start: hubs are created under the manager lock, so N concurrent
//!   subscribers produce exactly one `open_stream`.
//! - Each viewer has a bounded queue. A viewer that falls behind is dropped to
//!   the next keyframe instead of stalling the others or receiving a partial GOP.
//! - When the last viewer leaves, the stream drains for a grace period; a new
//!   viewer cancels the shutdown.
//! - If the upstream fails while viewers remain, it reconnects with backoff and
//!   viewers are told (`reconnecting`, `failed`). Unsupported streams and auth
//!   failures fail at once instead of retrying (camera lockout, SPEC §10.4).
//! - GOP caches are capped per stream and globally. Nothing is written to disk.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde::Serialize;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use super::fmp4::{self, SampleTiming};
use super::media::{MediaEvent, MediaSource, VideoParams};
use crate::events::{Event, EventBus};
use crate::providers::{ErrorCategory, ProviderError, StreamProfileId};
use crate::settings::SettingsHandle;

/// Packets queued per viewer before it counts as lagging (~10 s at 25 fps).
const SUBSCRIBER_QUEUE: usize = 256;
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
pub struct StreamKey {
    pub camera_id: Uuid,
    #[schema(value_type = String)]
    pub profile: StreamProfileId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    Starting,
    Running,
    Reconnecting,
    /// No viewers; closing after the grace period unless someone subscribes.
    Draining,
    Failed,
    Stopped,
}

/// The fMP4 initialization segment for the current codec parameters.
#[derive(Debug)]
pub struct InitSegment {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub data: Bytes,
}

#[derive(Debug)]
pub struct Fragment {
    pub keyframe: bool,
    /// `moof` + `mdat` for one access unit.
    pub data: Bytes,
}

/// What a viewer receives, in order.
#[derive(Debug, Clone)]
pub enum Packet {
    Init(Arc<InitSegment>),
    Fragment(Arc<Fragment>),
    State {
        state: StreamState,
        message: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("stream refused: {0}")]
    Refused(String),
    #[error("the stream manager is shutting down")]
    ShuttingDown,
}

/// Opens upstream sources. Abstracted so tests can count and control opens.
#[async_trait]
pub trait SourceOpener: Send + Sync {
    async fn open(
        &self,
        key: &StreamKey,
        cancel: CancellationToken,
    ) -> Result<Box<dyn MediaSource>, ProviderError>;
}

/// Admission hook for future bandwidth limits and priorities (SPEC §7.6).
/// Called before a new upstream stream starts. The MVP admits everything.
pub trait StreamPolicy: Send + Sync {
    fn admit(&self, key: &StreamKey, active: &[StreamInfo]) -> Result<(), String>;
}

pub struct AdmitAll;
impl StreamPolicy for AdmitAll {
    fn admit(&self, _: &StreamKey, _: &[StreamInfo]) -> Result<(), String> {
        Ok(())
    }
}

/// Per-stream metrics and state for the Streams page (SPEC §7.6).
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StreamInfo {
    pub camera_id: Uuid,
    #[schema(value_type = String)]
    pub profile: StreamProfileId,
    pub state: StreamState,
    pub subscribers: usize,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub started_at: OffsetDateTime,
    /// Time from the stream being requested to the first keyframe.
    pub startup_ms: Option<u64>,
    pub bytes_in_total: u64,
    pub bytes_out_total: u64,
    /// Upstream bytes per second (last second).
    pub in_bytes_per_sec: u64,
    /// Bytes per second sent to all viewers together (last second).
    pub out_bytes_per_sec: u64,
    pub reconnects: u64,
    pub errors: u64,
    /// Viewers dropped to the next keyframe for falling behind.
    pub lag_resyncs: u64,
    /// Bytes held in the GOP cache.
    pub buffer_bytes: usize,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Counters {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    reconnects: AtomicU64,
    errors: AtomicU64,
    lag_resyncs: AtomicU64,
    in_rate: AtomicU64,
    out_rate: AtomicU64,
    last_in: AtomicU64,
    last_out: AtomicU64,
}

struct Slot {
    id: u64,
    tx: mpsc::Sender<Packet>,
    waiting_for_key: bool,
}

struct HubState {
    state: StreamState,
    subscribers: Vec<Slot>,
    next_id: u64,
    init: Option<Arc<InitSegment>>,
    gop: Vec<Arc<Fragment>>,
    gop_bytes: usize,
    /// False after a cap was hit, until the next keyframe.
    caching: bool,
    drain_timer: Option<CancellationToken>,
    startup: Option<Duration>,
    last_error: Option<String>,
}

pub struct Hub {
    key: StreamKey,
    state: Mutex<HubState>,
    upstream: CancellationToken,
    counters: Counters,
    created: Instant,
    started_at: OffsetDateTime,
    manager: Weak<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Inner {
    hubs: Mutex<HashMap<StreamKey, Arc<Hub>>>,
    opener: Arc<dyn SourceOpener>,
    policy: Arc<dyn StreamPolicy>,
    settings: SettingsHandle,
    bus: EventBus,
    global_buffer: AtomicUsize,
    root: CancellationToken,
}

impl Inner {
    fn grace(&self) -> Duration {
        Duration::from_secs(self.settings.current().streaming.grace_period_secs)
    }

    fn stream_cap(&self) -> usize {
        usize::try_from(self.settings.current().streaming.stream_buffer_mb).unwrap_or(4)
            * 1024
            * 1024
    }

    fn global_cap(&self) -> usize {
        usize::try_from(self.settings.current().streaming.global_buffer_mb).unwrap_or(256)
            * 1024
            * 1024
    }
}

/// A viewer's handle. Dropping it unsubscribes.
pub struct Subscription {
    rx: mpsc::Receiver<Packet>,
    hub: Arc<Hub>,
    id: u64,
}

impl Subscription {
    /// The next packet; `None` when the stream was stopped.
    pub async fn next(&mut self) -> Option<Packet> {
        self.rx.recv().await
    }

    pub fn key(&self) -> &StreamKey {
        &self.hub.key
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(self.id);
    }
}

#[derive(Clone)]
pub struct StreamManager {
    inner: Arc<Inner>,
}

impl StreamManager {
    pub fn new(opener: Arc<dyn SourceOpener>, settings: SettingsHandle, bus: EventBus) -> Self {
        Self::with_policy(opener, settings, bus, Arc::new(AdmitAll))
    }

    pub fn with_policy(
        opener: Arc<dyn SourceOpener>,
        settings: SettingsHandle,
        bus: EventBus,
        policy: Arc<dyn StreamPolicy>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                hubs: Mutex::new(HashMap::new()),
                opener,
                policy,
                settings,
                bus,
                global_buffer: AtomicUsize::new(0),
                root: CancellationToken::new(),
            }),
        }
    }

    /// Subscribe to `key`, starting the upstream if needed.
    pub fn subscribe(&self, key: StreamKey) -> Result<Subscription, StreamError> {
        if self.inner.root.is_cancelled() {
            return Err(StreamError::ShuttingDown);
        }
        let hub = {
            let mut hubs = lock(&self.inner.hubs);
            match hubs.get(&key) {
                Some(hub) => Arc::clone(hub),
                None => {
                    let active: Vec<StreamInfo> = hubs.values().map(|h| h.info()).collect();
                    self.inner
                        .policy
                        .admit(&key, &active)
                        .map_err(StreamError::Refused)?;
                    let hub = Arc::new(Hub {
                        key: key.clone(),
                        state: Mutex::new(HubState {
                            state: StreamState::Starting,
                            subscribers: Vec::new(),
                            next_id: 0,
                            init: None,
                            gop: Vec::new(),
                            gop_bytes: 0,
                            caching: true,
                            drain_timer: None,
                            startup: None,
                            last_error: None,
                        }),
                        upstream: self.inner.root.child_token(),
                        counters: Counters::default(),
                        created: Instant::now(),
                        started_at: OffsetDateTime::now_utc(),
                        manager: Arc::downgrade(&self.inner),
                    });
                    hubs.insert(key.clone(), Arc::clone(&hub));
                    tracing::info!(camera_id = %key.camera_id, profile = %key.profile, "starting stream");
                    tokio::spawn(Arc::clone(&hub).run_upstream(Arc::clone(&self.inner.opener)));
                    hub
                }
            }
        };
        let (id, rx) = hub.subscribe();
        Ok(Subscription { rx, hub, id })
    }

    /// Snapshot of all active streams.
    pub fn list(&self) -> Vec<StreamInfo> {
        let mut v: Vec<StreamInfo> = lock(&self.inner.hubs).values().map(|h| h.info()).collect();
        v.sort_by(|a, b| (a.camera_id, &a.profile.0).cmp(&(b.camera_id, &b.profile.0)));
        v
    }

    /// Stop a stream now, disconnecting its viewers. Returns whether it existed.
    pub fn stop(&self, key: &StreamKey) -> bool {
        let hub = lock(&self.inner.hubs).remove(key);
        match hub {
            Some(hub) => {
                hub.stop(Some("stopped by an administrator".into()));
                true
            }
            None => false,
        }
    }

    /// Stop every stream (server shutdown).
    pub fn shutdown(&self) {
        self.inner.root.cancel();
        let hubs: Vec<_> = lock(&self.inner.hubs).drain().map(|(_, h)| h).collect();
        for hub in hubs {
            hub.stop(Some("server shutting down".into()));
        }
    }

    /// Total bytes held in GOP caches across all streams.
    pub fn buffer_bytes(&self) -> usize {
        self.inner.global_buffer.load(Ordering::Relaxed)
    }

    /// Update per-second rates and publish stream metrics (call every second).
    pub fn sample(&self) {
        let infos: Vec<StreamInfo> = {
            let hubs = lock(&self.inner.hubs);
            for hub in hubs.values() {
                let c = &hub.counters;
                let bin = c.bytes_in.load(Ordering::Relaxed);
                let bout = c.bytes_out.load(Ordering::Relaxed);
                c.in_rate.store(
                    bin - c.last_in.swap(bin, Ordering::Relaxed),
                    Ordering::Relaxed,
                );
                c.out_rate.store(
                    bout - c.last_out.swap(bout, Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            }
            hubs.values().map(|h| h.info()).collect()
        };
        self.inner
            .bus
            .publish(Event::StreamsUpdated { streams: infos });
    }

    /// Run [`StreamManager::sample`] every second until cancelled.
    pub async fn run_sampler(self, cancel: CancellationToken) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut was_empty = true;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = tick.tick() => {}
            }
            // Publish while streams exist, plus once after the last one ends.
            let empty = lock(&self.inner.hubs).is_empty();
            if !(empty && was_empty) {
                self.sample();
            }
            was_empty = empty;
        }
    }
}

impl Hub {
    fn info(&self) -> StreamInfo {
        let s = lock(&self.state);
        let c = &self.counters;
        StreamInfo {
            camera_id: self.key.camera_id,
            profile: self.key.profile.clone(),
            state: s.state,
            subscribers: s.subscribers.len(),
            codec: s.init.as_ref().map(|i| i.codec.clone()),
            width: s.init.as_ref().map(|i| i.width),
            height: s.init.as_ref().map(|i| i.height),
            started_at: self.started_at,
            startup_ms: s
                .startup
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            bytes_in_total: c.bytes_in.load(Ordering::Relaxed),
            bytes_out_total: c.bytes_out.load(Ordering::Relaxed),
            in_bytes_per_sec: c.in_rate.load(Ordering::Relaxed),
            out_bytes_per_sec: c.out_rate.load(Ordering::Relaxed),
            reconnects: c.reconnects.load(Ordering::Relaxed),
            errors: c.errors.load(Ordering::Relaxed),
            lag_resyncs: c.lag_resyncs.load(Ordering::Relaxed),
            buffer_bytes: s.gop_bytes,
            last_error: s.last_error.clone(),
        }
    }

    fn subscribe(&self) -> (u64, mpsc::Receiver<Packet>) {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE);
        let mut s = lock(&self.state);
        let id = s.next_id;
        s.next_id += 1;
        if let Some(timer) = s.drain_timer.take() {
            timer.cancel();
            s.state = if s.init.is_some() {
                StreamState::Running
            } else {
                StreamState::Starting
            };
            tracing::debug!(camera_id = %self.key.camera_id, "viewer returned; drain cancelled");
        }
        // Current state, then init + cached GOP so playback starts at once.
        let _ = tx.try_send(Packet::State {
            state: s.state,
            message: s.last_error.clone(),
        });
        let mut waiting_for_key = true;
        if let Some(init) = &s.init {
            let _ = tx.try_send(Packet::Init(Arc::clone(init)));
            if s.caching && !s.gop.is_empty() {
                let mut sent = 0u64;
                for f in &s.gop {
                    if tx.try_send(Packet::Fragment(Arc::clone(f))).is_ok() {
                        sent += f.data.len() as u64;
                    }
                }
                self.counters.bytes_out.fetch_add(sent, Ordering::Relaxed);
                waiting_for_key = false;
            }
        }
        s.subscribers.push(Slot {
            id,
            tx,
            waiting_for_key,
        });
        (id, rx)
    }

    fn unsubscribe(self: &Arc<Self>, id: u64) {
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let mut s = lock(&self.state);
        s.subscribers.retain(|slot| slot.id != id);
        if !s.subscribers.is_empty()
            || matches!(s.state, StreamState::Stopped | StreamState::Draining)
        {
            return;
        }
        let grace = manager.grace();
        s.state = StreamState::Draining;
        let timer = CancellationToken::new();
        s.drain_timer = Some(timer.clone());
        drop(s);
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            tokio::select! {
                () = timer.cancelled() => {}
                () = tokio::time::sleep(grace) => hub.finish_drain(&manager),
            }
        });
    }

    /// Grace period over: stop if still unwatched. Lock order: manager → hub.
    fn finish_drain(self: &Arc<Self>, manager: &Inner) {
        let mut hubs = lock(&manager.hubs);
        let still_ours = hubs.get(&self.key).is_some_and(|h| Arc::ptr_eq(h, self));
        let empty = lock(&self.state).subscribers.is_empty();
        if still_ours && empty {
            hubs.remove(&self.key);
            drop(hubs);
            tracing::info!(camera_id = %self.key.camera_id, profile = %self.key.profile, "stream closed after grace period");
            self.stop(None);
        }
    }

    fn stop(&self, message: Option<String>) {
        self.upstream.cancel();
        let mut s = lock(&self.state);
        s.state = StreamState::Stopped;
        if let Some(t) = s.drain_timer.take() {
            t.cancel();
        }
        for slot in &s.subscribers {
            let _ = slot.tx.try_send(Packet::State {
                state: StreamState::Stopped,
                message: message.clone(),
            });
        }
        // Dropping the senders ends every viewer's stream.
        s.subscribers.clear();
        self.release_gop(&mut s);
    }

    fn release_gop(&self, s: &mut HubState) {
        if let Some(m) = self.manager.upgrade() {
            m.global_buffer.fetch_sub(s.gop_bytes, Ordering::Relaxed);
        }
        s.gop.clear();
        s.gop_bytes = 0;
    }

    fn set_state(&self, state: StreamState, message: Option<String>) {
        let mut s = lock(&self.state);
        if matches!(s.state, StreamState::Stopped) {
            return;
        }
        // Draining stays draining until a viewer returns.
        if s.state == StreamState::Draining && state == StreamState::Running {
            return;
        }
        s.state = state;
        if message.is_some() {
            s.last_error = message.clone();
        }
        for slot in &s.subscribers {
            let _ = slot.tx.try_send(Packet::State {
                state,
                message: message.clone(),
            });
        }
    }

    fn publish_init(&self, params: &VideoParams) {
        let init = Arc::new(InitSegment {
            codec: params.rfc6381.clone(),
            width: params.width,
            height: params.height,
            data: fmp4::init_segment(params.width, params.height, &params.sample_entry).into(),
        });
        let mut s = lock(&self.state);
        if s.init.as_ref().is_some_and(|i| i.data == init.data) {
            return; // same parameters (e.g. after a reconnect)
        }
        s.init = Some(Arc::clone(&init));
        // Old GOP doesn't decode with new parameters; viewers wait for a keyframe.
        self.release_gop(&mut s);
        for slot in &mut s.subscribers {
            let _ = slot.tx.try_send(Packet::Init(Arc::clone(&init)));
            slot.waiting_for_key = true;
        }
    }

    fn distribute(&self, frag: Fragment) {
        let frag = Arc::new(frag);
        let Some(manager) = self.manager.upgrade() else {
            return;
        };
        let mut s = lock(&self.state);
        if s.state == StreamState::Stopped {
            return;
        }
        let len = frag.data.len();

        // GOP cache: restart at each keyframe; drop it if a cap would be exceeded
        // (a truncated GOP would break decoding for new viewers).
        if frag.keyframe {
            self.release_gop(&mut s);
            s.caching = true;
            if s.startup.is_none() {
                s.startup = Some(self.created.elapsed());
            }
        }
        if s.caching {
            let over_stream = s.gop_bytes + len > manager.stream_cap();
            let over_global =
                manager.global_buffer.load(Ordering::Relaxed) + len > manager.global_cap();
            if over_stream || over_global {
                self.release_gop(&mut s);
                s.caching = false;
            } else {
                manager.global_buffer.fetch_add(len, Ordering::Relaxed);
                s.gop_bytes += len;
                s.gop.push(Arc::clone(&frag));
            }
        }

        let mut sent = 0u64;
        let mut lagged = 0u64;
        s.subscribers.retain_mut(|slot| {
            if slot.waiting_for_key && !frag.keyframe {
                return true;
            }
            match slot.tx.try_send(Packet::Fragment(Arc::clone(&frag))) {
                Ok(()) => {
                    slot.waiting_for_key = false;
                    sent += len as u64;
                    true
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if !slot.waiting_for_key {
                        lagged += 1;
                    }
                    slot.waiting_for_key = true;
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
        self.counters.bytes_out.fetch_add(sent, Ordering::Relaxed);
        self.counters
            .lag_resyncs
            .fetch_add(lagged, Ordering::Relaxed);
    }

    /// The upstream loop: open, remux, distribute; reconnect with backoff.
    async fn run_upstream(self: Arc<Self>, opener: Arc<dyn SourceOpener>) {
        let mut backoff = Duration::from_secs(1);
        let mut seq: u32 = 0;
        // Keeps decode times continuous across reconnects.
        let mut next_dts: u64 = 0;
        let mut first = true;

        loop {
            if self.upstream.is_cancelled() {
                return;
            }
            if !first {
                self.counters.reconnects.fetch_add(1, Ordering::Relaxed);
            }
            let opened = tokio::select! {
                () = self.upstream.cancelled() => return,
                r = opener.open(&self.key, self.upstream.child_token()) => r,
            };
            let mut source = match opened {
                Ok(s) => s,
                Err(e) => {
                    self.counters.errors.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(camera_id = %self.key.camera_id, error = %e, "cannot open stream");
                    if matches!(
                        e.category(),
                        ErrorCategory::Unsupported | ErrorCategory::AuthFailed
                    ) {
                        self.fail(e.to_string());
                        return;
                    }
                    self.set_state(StreamState::Reconnecting, Some(e.to_string()));
                    tokio::select! {
                        () = self.upstream.cancelled() => return,
                        () = tokio::time::sleep(backoff) => {}
                    }
                    backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
                    first = false;
                    continue;
                }
            };
            first = false;

            let mut base: Option<i64> = None; // first dts of this session
            let session_start = next_dts;
            let mut last_duration: u32 = 3600;
            let mut prev_dts: Option<i64> = None;
            let result = loop {
                let event = tokio::select! {
                    () = self.upstream.cancelled() => return,
                    e = source.next() => e,
                };
                match event {
                    Ok(Some(MediaEvent::Params(p))) => self.publish_init(&p),
                    Ok(Some(MediaEvent::Frame(f))) => {
                        if lock(&self.state).init.is_none() {
                            continue; // no parameters yet
                        }
                        let b = *base.get_or_insert(f.dts);
                        if let Some(prev) = prev_dts {
                            last_duration =
                                u32::try_from(f.dts - prev).unwrap_or(last_duration).max(1);
                        }
                        prev_dts = Some(f.dts);
                        let decode_time = session_start + u64::try_from(f.dts - b).unwrap_or(0);
                        next_dts = decode_time + u64::from(last_duration);
                        self.counters
                            .bytes_in
                            .fetch_add(f.data.len() as u64, Ordering::Relaxed);
                        seq = seq.wrapping_add(1);
                        let data = fmp4::fragment(
                            seq,
                            SampleTiming {
                                decode_time,
                                duration: last_duration,
                                composition_offset: i32::try_from(f.pts - f.dts).unwrap_or(0),
                            },
                            f.keyframe,
                            &f.data,
                        );
                        if f.keyframe {
                            backoff = Duration::from_secs(1);
                            let state = lock(&self.state).state;
                            if matches!(state, StreamState::Starting | StreamState::Reconnecting) {
                                self.set_state(StreamState::Running, None);
                            }
                        }
                        self.distribute(Fragment {
                            keyframe: f.keyframe,
                            data: data.into(),
                        });
                    }
                    Ok(None) => break None,
                    Err(e) => break Some(e),
                }
            };
            if let Some(e) = &result {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(camera_id = %self.key.camera_id, error = %e, "stream interrupted");
            }
            let message = result.map_or_else(
                || "the camera ended the stream".to_owned(),
                |e| e.to_string(),
            );
            self.set_state(StreamState::Reconnecting, Some(message));
            tokio::select! {
                () = self.upstream.cancelled() => return,
                () = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(MAX_RECONNECT_BACKOFF);
        }
    }

    /// Permanent failure: tell viewers and remove the hub so a later request
    /// tries again.
    fn fail(self: &Arc<Self>, message: String) {
        self.set_state(StreamState::Failed, Some(message.clone()));
        if let Some(manager) = self.manager.upgrade() {
            let mut hubs = lock(&manager.hubs);
            if hubs.get(&self.key).is_some_and(|h| Arc::ptr_eq(h, self)) {
                hubs.remove(&self.key);
            }
        }
        self.stop(Some(message));
    }
}

#[cfg(test)]
mod tests;
