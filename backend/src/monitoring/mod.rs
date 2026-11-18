//! Camera status monitoring.
//!
//! Phase 2 version of the polling service (SPEC §10.2): a fixed interval with
//! jitter, at most one check in flight per camera, a global concurrency cap, a
//! per-check timeout, and a failure threshold before a camera is marked offline.
//! Phase 4 extends this behind the same [`PollingPolicy`] (offline backoff,
//! retries, admin-editable settings).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::cameras::{self, CameraState, CameraStatusView, MonitorTarget, Scheme};
use crate::config::MonitoringConfig;
use crate::events::{Event, EventBus};
use crate::providers::{
    CameraProvider, ErrorCategory, ProviderError, ProviderFactory, ProviderTarget,
};

/// Decides how often each camera is checked. In the MVP active and idle cameras
/// share one policy; streamed cameras can get their own interval later.
#[derive(Debug, Clone)]
pub struct PollingPolicy {
    pub interval: Duration,
    /// Fraction of `interval` to randomize by, e.g. 0.2 for ±20%.
    pub jitter: f64,
    pub failure_threshold: u32,
    pub timeout: Duration,
    pub max_in_flight: usize,
}

impl From<&MonitoringConfig> for PollingPolicy {
    fn from(c: &MonitoringConfig) -> Self {
        Self {
            interval: Duration::from_secs(c.interval_secs),
            jitter: f64::from(c.jitter_percent) / 100.0,
            failure_threshold: c.failure_threshold,
            timeout: Duration::from_secs(c.timeout_secs),
            max_in_flight: c.max_concurrent,
        }
    }
}

impl PollingPolicy {
    /// `interval` randomized by ±`jitter`, using `unit` in [0, 1).
    pub fn next_delay(&self, unit: f64) -> Duration {
        let spread = (unit.clamp(0.0, 1.0) * 2.0 - 1.0) * self.jitter;
        self.interval.mul_f64((1.0 + spread).max(0.0))
    }
}

/// What a single check produced.
#[derive(Debug, Clone)]
pub enum CheckOutcome {
    Success { response_time_ms: u32 },
    Failure(ProviderError),
}

/// Pure state transition: the new status, plus `(from, to)` if the state changed.
pub fn apply(
    prev: &CameraStatusView,
    outcome: &CheckOutcome,
    now: OffsetDateTime,
    failure_threshold: u32,
) -> (CameraStatusView, Option<(CameraState, CameraState)>) {
    let mut next = prev.clone();
    next.last_checked_at = Some(now);
    match outcome {
        CheckOutcome::Success { response_time_ms } => {
            next.state = CameraState::Online;
            next.last_success_at = Some(now);
            next.response_time_ms = Some(*response_time_ms);
            next.consecutive_failures = 0;
            next.error_category = None;
            next.error_message = None;
        }
        CheckOutcome::Failure(err) if err.category() == ErrorCategory::Unsupported => {
            // Nothing to monitor with (e.g. no provider yet): status is unknown,
            // not a failure.
            next.state = CameraState::Unknown;
            next.response_time_ms = None;
            next.consecutive_failures = 0;
            next.error_category = Some(ErrorCategory::Unsupported);
            next.error_message = Some(err.to_string());
        }
        CheckOutcome::Failure(err) => {
            next.consecutive_failures = prev.consecutive_failures.saturating_add(1);
            next.last_failure_at = Some(now);
            next.response_time_ms = None;
            next.error_category = Some(err.category());
            next.error_message = Some(err.to_string());
            if next.consecutive_failures >= failure_threshold.max(1) {
                next.state = CameraState::Offline;
            }
        }
    }
    let transition = (next.state != prev.state).then(|| {
        next.last_change_at = Some(now);
        (prev.state, next.state)
    });
    (next, transition)
}

fn target_of(t: &MonitorTarget) -> ProviderTarget {
    ProviderTarget {
        host: t.host.clone(),
        port: t.port,
        https: t.scheme == Scheme::Https,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Fetch identity, capabilities and stream profiles and store them. Unsupported
/// capability or stream APIs keep whatever was stored before.
pub async fn refresh_identity(
    db: &SqlitePool,
    provider: &dyn CameraProvider,
    id: Uuid,
    timeout: Duration,
) -> Result<(), RefreshError> {
    let bounded = |what: &'static str| {
        ProviderError::Timeout(format!("{what} took longer than {}s", timeout.as_secs()))
    };
    let probe = tokio::time::timeout(timeout, provider.probe())
        .await
        .map_err(|_| bounded("probe"))??;
    let caps = match tokio::time::timeout(timeout, provider.get_capabilities()).await {
        Ok(Ok(c)) => Some(c),
        Ok(Err(e)) if e.category() == ErrorCategory::Unsupported => None,
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(bounded("capabilities").into()),
    };
    let profiles = match tokio::time::timeout(timeout, provider.get_stream_info()).await {
        Ok(Ok(p)) => Some(p),
        Ok(Err(e)) if e.category() == ErrorCategory::Unsupported => None,
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(bounded("stream info").into()),
    };
    cameras::save_identity(db, id, &probe.device, caps.as_ref(), profiles.as_deref()).await?;
    Ok(())
}

struct Track {
    provider: Arc<dyn CameraProvider>,
    updated_at: OffsetDateTime,
    next_due: Instant,
    in_flight: Arc<AtomicBool>,
}

/// Tiny PRNG for jitter (no security requirement).
struct Jitter(Mutex<u64>);

impl Jitter {
    fn new() -> Self {
        let seed = OffsetDateTime::now_utc().unix_timestamp_nanos() as u64 | 1;
        Self(Mutex::new(seed))
    }

    fn unit(&self) -> f64 {
        let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // xorshift64*
        *s ^= *s >> 12;
        *s ^= *s << 25;
        *s ^= *s >> 27;
        let v = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub struct Monitor {
    db: SqlitePool,
    factory: ProviderFactory,
    bus: EventBus,
    policy: PollingPolicy,
    tracks: HashMap<Uuid, Track>,
    permits: Arc<Semaphore>,
    jitter: Arc<Jitter>,
    /// How often the registry is re-read to pick up added/removed cameras.
    tick: Duration,
}

impl Monitor {
    pub fn new(
        db: SqlitePool,
        factory: ProviderFactory,
        bus: EventBus,
        policy: PollingPolicy,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(policy.max_in_flight.max(1)));
        Self {
            db,
            factory,
            bus,
            policy,
            tracks: HashMap::new(),
            permits,
            jitter: Arc::new(Jitter::new()),
            tick: Duration::from_millis(500),
        }
    }

    /// Run until cancelled. Errors reading the registry are logged and retried on
    /// the next tick; they never stop the service.
    pub async fn run(mut self, cancel: CancellationToken) {
        tracing::info!(
            interval_secs = self.policy.interval.as_secs(),
            "camera status monitor started"
        );
        let mut ticker = tokio::time::interval(self.tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            if let Err(err) = self.tick_once(&cancel).await {
                tracing::warn!(error = %err, "status monitor could not read cameras");
            }
        }
        tracing::info!("camera status monitor stopped");
    }

    async fn tick_once(&mut self, cancel: &CancellationToken) -> Result<(), sqlx::Error> {
        let targets = cameras::monitor_targets(&self.db).await?;
        let now = Instant::now();
        self.tracks
            .retain(|id, _| targets.iter().any(|t| t.id == *id));

        for target in targets {
            let track = match self.tracks.get_mut(&target.id) {
                Some(track) if track.updated_at == target.updated_at => track,
                _ => {
                    // New or reconfigured camera: (re)build its provider and spread
                    // first checks over one interval so a big fleet doesn't start
                    // in lockstep.
                    let first = self.policy.interval.mul_f64(self.jitter.unit());
                    self.tracks.insert(
                        target.id,
                        Track {
                            provider: self.factory.create(&target_of(&target)),
                            updated_at: target.updated_at,
                            next_due: now + first.min(Duration::from_secs(2)),
                            in_flight: Arc::new(AtomicBool::new(false)),
                        },
                    );
                    self.tracks
                        .get_mut(&target.id)
                        .unwrap_or_else(|| unreachable!("just inserted"))
                }
            };

            if track.next_due > now || track.in_flight.load(Ordering::Acquire) {
                continue;
            }
            let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
                break; // at capacity; remaining cameras wait for the next tick
            };
            track.in_flight.store(true, Ordering::Release);
            track.next_due = now + self.policy.next_delay(self.jitter.unit());

            let ctx = CheckContext {
                db: self.db.clone(),
                bus: self.bus.clone(),
                policy: self.policy.clone(),
                provider: Arc::clone(&track.provider),
                in_flight: Arc::clone(&track.in_flight),
            };
            let cancel = cancel.child_token();
            tokio::spawn(async move {
                let _permit = permit;
                tokio::select! {
                    () = cancel.cancelled() => {}
                    () = ctx.check(target) => {}
                }
            });
        }
        Ok(())
    }
}

struct CheckContext {
    db: SqlitePool,
    bus: EventBus,
    policy: PollingPolicy,
    provider: Arc<dyn CameraProvider>,
    in_flight: Arc<AtomicBool>,
}

impl CheckContext {
    async fn check(self, target: MonitorTarget) {
        let outcome =
            match tokio::time::timeout(self.policy.timeout, self.provider.get_status()).await {
                Ok(Ok(s)) => CheckOutcome::Success {
                    response_time_ms: s.response_time_ms.max(1),
                },
                Ok(Err(e)) => CheckOutcome::Failure(e),
                Err(_) => CheckOutcome::Failure(ProviderError::Timeout(format!(
                    "no answer within {}s",
                    self.policy.timeout.as_secs()
                ))),
            };
        let (status, transition) = apply(
            &target.status,
            &outcome,
            OffsetDateTime::now_utc(),
            self.policy.failure_threshold,
        );

        if let Some((from, to)) = transition {
            tracing::info!(
                camera_id = %target.id,
                from = from.as_str(),
                to = to.as_str(),
                error_category = ?status.error_category,
                "camera status changed"
            );
        }
        if let Err(err) = cameras::write_status(&self.db, target.id, &status, transition).await {
            tracing::warn!(camera_id = %target.id, error = %err, "failed to store camera status");
        }

        // Fetch identity the first time a camera is reachable, and again whenever
        // it comes back online (firmware may have changed).
        let became_online = transition.is_some_and(|(_, to)| to == CameraState::Online);
        if status.state == CameraState::Online && (became_online || !target.has_identity) {
            match refresh_identity(&self.db, &*self.provider, target.id, self.policy.timeout).await
            {
                Ok(()) => {
                    if let Ok(Some(cam)) = cameras::get(&self.db, target.id).await {
                        self.bus.publish(Event::CameraChanged {
                            camera: cam.summary,
                        });
                    }
                }
                Err(err) => {
                    tracing::debug!(camera_id = %target.id, error = %err, "identity refresh failed")
                }
            }
        }

        self.bus.publish(Event::CameraStatus {
            camera_id: target.id,
            status,
        });
        self.in_flight.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let policy = PollingPolicy {
            interval: Duration::from_secs(10),
            jitter: 0.2,
            failure_threshold: 3,
            timeout: Duration::from_secs(5),
            max_in_flight: 4,
        };
        assert_eq!(policy.next_delay(0.0), Duration::from_secs(8));
        assert_eq!(policy.next_delay(0.5), Duration::from_secs(10));
        let j = Jitter::new();
        for _ in 0..1000 {
            let d = policy.next_delay(j.unit());
            assert!(
                d >= Duration::from_secs(8) && d <= Duration::from_secs(12),
                "{d:?}"
            );
        }
    }

    #[test]
    fn success_marks_online_and_records_transition_once() {
        let unknown = CameraStatusView::default();
        let ok = CheckOutcome::Success {
            response_time_ms: 30,
        };
        let (s1, t1) = apply(&unknown, &ok, now(), 3);
        assert_eq!(s1.state, CameraState::Online);
        assert_eq!(t1, Some((CameraState::Unknown, CameraState::Online)));
        let (s2, t2) = apply(&s1, &ok, now(), 3);
        assert_eq!(s2.state, CameraState::Online);
        assert_eq!(t2, None, "no transition when nothing changed");
    }

    #[test]
    fn offline_only_after_threshold_failures() {
        let fail = CheckOutcome::Failure(ProviderError::Unreachable("x".into()));
        let mut status = apply(
            &CameraStatusView::default(),
            &CheckOutcome::Success {
                response_time_ms: 5,
            },
            now(),
            3,
        )
        .0;
        for i in 1..=2 {
            let (s, t) = apply(&status, &fail, now(), 3);
            assert_eq!(
                s.state,
                CameraState::Online,
                "still online after {i} failure(s)"
            );
            assert_eq!(t, None);
            status = s;
        }
        let (s, t) = apply(&status, &fail, now(), 3);
        assert_eq!(s.state, CameraState::Offline);
        assert_eq!(t, Some((CameraState::Online, CameraState::Offline)));
        assert_eq!(s.error_category, Some(ErrorCategory::Unreachable));
        assert_eq!(s.consecutive_failures, 3);
    }

    #[test]
    fn auth_failure_counts_and_carries_category() {
        let fail = CheckOutcome::Failure(ProviderError::AuthFailed("401".into()));
        let mut s = CameraStatusView::default();
        for _ in 0..3 {
            s = apply(&s, &fail, now(), 3).0;
        }
        assert_eq!(s.state, CameraState::Offline);
        assert_eq!(s.error_category, Some(ErrorCategory::AuthFailed));
    }

    #[test]
    fn unsupported_is_unknown_not_failure() {
        let online = apply(
            &CameraStatusView::default(),
            &CheckOutcome::Success {
                response_time_ms: 5,
            },
            now(),
            3,
        )
        .0;
        let (s, t) = apply(
            &online,
            &CheckOutcome::Failure(ProviderError::unsupported("x")),
            now(),
            3,
        );
        assert_eq!(s.state, CameraState::Unknown);
        assert_eq!(s.consecutive_failures, 0);
        assert_eq!(t, Some((CameraState::Online, CameraState::Unknown)));
    }
}
