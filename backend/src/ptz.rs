//! Interactive pan/tilt/zoom (issue #31, ADR 0013).
//!
//! The UI sends a move when a control is pressed, repeats it every
//! [`KEEPALIVE`] while it's held, and sends a stop on release. Cameras keep
//! moving until they're told to stop, so a watchdog stops any camera whose
//! move hasn't been repeated within [`WATCHDOG`] (a closed tab, a lost
//! connection). Repeats of the same move aren't sent to the camera again and
//! don't count as a new gesture: a held button is one audit entry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::task::JoinSet;
use tokio::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::providers::{CameraProvider, ErrorCategory, ProviderError, PtzMove};

/// How often the UI repeats a held move.
pub const KEEPALIVE: Duration = Duration::from_millis(400);
/// A move not repeated for this long is stopped by the server.
pub const WATCHDOG: Duration = Duration::from_millis(1500);
/// Longest wait for one camera to take a command.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
/// Commands per user: burst size and sustained rate per second. A held
/// control costs 1000/400 = 2.5 per second.
const BURST: f64 = 20.0;
const PER_SECOND: f64 = 10.0;

struct Active {
    mv: PtzMove,
    last_seen: Instant,
    /// Reused for the whole gesture, so Digest auth is negotiated once.
    provider: Arc<dyn CameraProvider>,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

#[derive(Default)]
struct Inner {
    active: Mutex<HashMap<Uuid, Active>>,
    buckets: Mutex<HashMap<Uuid, Bucket>>,
}

/// Shared PTZ state; cheap to clone.
#[derive(Clone, Default)]
pub struct PtzControl {
    inner: Arc<Inner>,
}

/// One camera to move.
pub struct Target {
    pub camera_id: Uuid,
    pub provider: Arc<dyn CameraProvider>,
}

/// What happened for one camera.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PtzCameraResult {
    pub camera_id: Uuid,
    pub ok: bool,
    pub error_category: Option<ErrorCategory>,
    /// Short, non-sensitive description of the error.
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PtzOutcome {
    pub results: Vec<PtzCameraResult>,
    /// `true` when this command started or changed a movement (it's audited);
    /// `false` for repeats and stops.
    #[serde(skip)]
    pub new_gesture: bool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PtzControl {
    /// Take one command from `user`'s budget; `Err(seconds)` to wait if spent.
    pub fn admit(&self, user: Uuid) -> Result<(), u64> {
        let now = Instant::now();
        let mut buckets = lock(&self.inner.buckets);
        let b = buckets.entry(user).or_insert(Bucket {
            tokens: BURST,
            at: now,
        });
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * PER_SECOND).min(BURST);
        b.at = now;
        if b.tokens < 1.0 {
            return Err(1);
        }
        b.tokens -= 1.0;
        Ok(())
    }

    /// Whether `camera` is currently moving (a move not yet stopped).
    pub fn is_moving(&self, camera: Uuid) -> bool {
        lock(&self.inner.active).contains_key(&camera)
    }

    /// Apply `mv` to every target: new or changed moves go to the camera,
    /// repeats only keep the movement alive, and stops always go out.
    pub async fn command(&self, targets: Vec<Target>, mv: PtzMove) -> PtzOutcome {
        let now = Instant::now();
        let mut new_gesture = false;
        let mut send: Vec<(Uuid, Arc<dyn CameraProvider>)> = Vec::new();
        let mut results = Vec::new();
        {
            let mut active = lock(&self.inner.active);
            for t in targets {
                let prev = active.get_mut(&t.camera_id);
                if mv.is_stop() {
                    let provider = prev.map_or(t.provider, |a| Arc::clone(&a.provider));
                    active.remove(&t.camera_id);
                    send.push((t.camera_id, provider));
                    continue;
                }
                match prev {
                    Some(a) if a.mv == mv => {
                        a.last_seen = now;
                        results.push(ok(t.camera_id));
                    }
                    Some(a) => {
                        a.mv = mv;
                        a.last_seen = now;
                        new_gesture = true;
                        send.push((t.camera_id, Arc::clone(&a.provider)));
                    }
                    None => {
                        new_gesture = true;
                        active.insert(
                            t.camera_id,
                            Active {
                                mv,
                                last_seen: now,
                                provider: Arc::clone(&t.provider),
                            },
                        );
                        self.spawn_watchdog(t.camera_id);
                        send.push((t.camera_id, t.provider));
                    }
                }
            }
        }

        let mut set = JoinSet::new();
        for (id, provider) in send {
            set.spawn(async move {
                let r = match tokio::time::timeout(COMMAND_TIMEOUT, provider.ptz_move(mv)).await {
                    Ok(r) => r,
                    Err(_) => Err(ProviderError::Timeout(
                        "the camera didn't take the command".into(),
                    )),
                };
                (id, r)
            });
        }
        while let Some(joined) = set.join_next().await {
            let Ok((id, r)) = joined else { continue };
            match r {
                Ok(()) => results.push(ok(id)),
                Err(e) => {
                    // A camera that refused doesn't need stopping later.
                    if !mv.is_stop() {
                        lock(&self.inner.active).remove(&id);
                    }
                    results.push(PtzCameraResult {
                        camera_id: id,
                        ok: false,
                        error_category: Some(e.category()),
                        message: Some(e.to_string()),
                    });
                }
            }
        }
        results.sort_by_key(|r| r.camera_id);
        PtzOutcome {
            results,
            new_gesture,
        }
    }

    /// Stop `camera` if its move isn't repeated in time.
    fn spawn_watchdog(&self, camera: Uuid) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(KEEPALIVE).await;
                let stale = {
                    let mut active = lock(&inner.active);
                    match active.get(&camera) {
                        None => return,
                        Some(a) if a.last_seen.elapsed() >= WATCHDOG => {
                            active.remove(&camera).map(|a| a.provider)
                        }
                        Some(_) => None,
                    }
                };
                if let Some(provider) = stale {
                    tracing::warn!(camera_id = %camera, "PTZ move not renewed; stopping the camera");
                    let r = tokio::time::timeout(COMMAND_TIMEOUT, provider.ptz_move(PtzMove::STOP))
                        .await;
                    if !matches!(r, Ok(Ok(()))) {
                        tracing::warn!(camera_id = %camera, "couldn't stop the camera after a lost PTZ move");
                    }
                    return;
                }
            }
        });
    }
}

fn ok(camera_id: Uuid) -> PtzCameraResult {
    PtzCameraResult {
        camera_id,
        ok: true,
        error_category: None,
        message: None,
    }
}

/// Words for the audit log, e.g. "pan left, zoom in".
pub fn describe(mv: PtzMove) -> String {
    if mv.is_stop() {
        return "stop".into();
    }
    let mut parts = Vec::new();
    match mv.pan.signum() {
        1 => parts.push("pan right"),
        -1 => parts.push("pan left"),
        _ => {}
    }
    match mv.tilt.signum() {
        1 => parts.push("tilt up"),
        -1 => parts.push("tilt down"),
        _ => {}
    }
    match mv.zoom.signum() {
        1 => parts.push("zoom in"),
        -1 => parts.push("zoom out"),
        _ => {}
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records PTZ commands; everything else is unused here.
    #[derive(Default)]
    struct Recorder {
        moves: Mutex<Vec<PtzMove>>,
        calls: AtomicUsize,
        refuse: bool,
    }

    #[async_trait]
    impl CameraProvider for Recorder {
        async fn probe(&self) -> Result<crate::providers::ProbeResult, ProviderError> {
            unimplemented!()
        }
        async fn get_device_info(&self) -> Result<crate::providers::DeviceInfo, ProviderError> {
            unimplemented!()
        }
        async fn get_status(&self) -> Result<crate::providers::CameraStatus, ProviderError> {
            unimplemented!()
        }
        async fn get_capabilities(
            &self,
        ) -> Result<crate::providers::CameraCapabilities, ProviderError> {
            unimplemented!()
        }
        async fn get_stream_info(
            &self,
        ) -> Result<Vec<crate::providers::StreamProfile>, ProviderError> {
            unimplemented!()
        }
        async fn open_stream(
            &self,
            _: &crate::providers::StreamProfileId,
            _: tokio_util::sync::CancellationToken,
        ) -> Result<Box<dyn crate::providers::MediaSource>, ProviderError> {
            unimplemented!()
        }
        async fn restart(&self) -> Result<(), ProviderError> {
            unimplemented!()
        }
        async fn ptz_move(&self, mv: PtzMove) -> Result<(), ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.refuse {
                return Err(ProviderError::unsupported("pan/tilt/zoom"));
            }
            lock(&self.moves).push(mv);
            Ok(())
        }
    }

    const LEFT: PtzMove = PtzMove {
        pan: -50,
        tilt: 0,
        zoom: 0,
    };
    const ZOOM_IN: PtzMove = PtzMove {
        pan: 0,
        tilt: 0,
        zoom: 50,
    };

    fn target(id: u128, p: &Arc<Recorder>) -> Target {
        Target {
            camera_id: Uuid::from_u128(id),
            provider: Arc::clone(p) as Arc<dyn CameraProvider>,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_move_is_one_gesture_sent_once_then_stopped() {
        let ptz = PtzControl::default();
        let (a, b) = (Arc::new(Recorder::default()), Arc::new(Recorder::default()));
        let both = || vec![target(1, &a), target(2, &b)];

        let first = ptz.command(both(), LEFT).await;
        assert!(first.new_gesture);
        assert!(first.results.iter().all(|r| r.ok));
        for _ in 0..3 {
            tokio::time::sleep(KEEPALIVE).await;
            let again = ptz.command(both(), LEFT).await;
            assert!(!again.new_gesture, "repeats aren't new gestures");
        }
        assert!(ptz.is_moving(Uuid::from_u128(1)));
        let changed = ptz.command(both(), ZOOM_IN).await;
        assert!(changed.new_gesture, "a different move is");
        let stop = ptz.command(both(), PtzMove::STOP).await;
        assert!(!stop.new_gesture);
        assert_eq!(*lock(&a.moves), [LEFT, ZOOM_IN, PtzMove::STOP]);
        assert_eq!(*lock(&b.moves), [LEFT, ZOOM_IN, PtzMove::STOP]);
        assert!(!ptz.is_moving(Uuid::from_u128(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_move_that_is_not_renewed_is_stopped_by_the_watchdog() {
        let ptz = PtzControl::default();
        let a = Arc::new(Recorder::default());
        ptz.command(vec![target(1, &a)], LEFT).await;
        tokio::time::sleep(WATCHDOG + KEEPALIVE * 2).await;
        assert_eq!(*lock(&a.moves), [LEFT, PtzMove::STOP]);
        assert!(!ptz.is_moving(Uuid::from_u128(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusing_camera_is_reported_and_not_kept_moving() {
        let ptz = PtzControl::default();
        let fixed = Arc::new(Recorder {
            refuse: true,
            ..Recorder::default()
        });
        let out = ptz.command(vec![target(1, &fixed)], LEFT).await;
        assert!(!out.results[0].ok);
        assert_eq!(
            out.results[0].error_category,
            Some(ErrorCategory::Unsupported)
        );
        assert!(!ptz.is_moving(Uuid::from_u128(1)));
        tokio::time::sleep(WATCHDOG * 2).await;
        assert_eq!(fixed.calls.load(Ordering::SeqCst), 1, "no watchdog stop");
    }

    #[tokio::test(start_paused = true)]
    async fn commands_are_rate_limited_per_user() {
        let ptz = PtzControl::default();
        let (u, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        for _ in 0..20 {
            ptz.admit(u).unwrap();
        }
        assert!(ptz.admit(u).is_err(), "burst spent");
        assert!(ptz.admit(other).is_ok(), "per user");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(ptz.admit(u).is_ok(), "refills at 10 per second");
    }

    #[test]
    fn moves_are_described_in_words() {
        assert_eq!(describe(LEFT), "pan left");
        assert_eq!(
            describe(PtzMove {
                pan: 10,
                tilt: -10,
                zoom: 30
            }),
            "pan right, tilt down, zoom in"
        );
        assert_eq!(describe(PtzMove::STOP), "stop");
    }
}
