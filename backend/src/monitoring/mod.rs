//! Camera status monitoring: the polling service (SPEC §10.2).
//!
//! - Each monitored camera is checked every `interval` ± jitter, with at most
//!   one check in flight per camera and a global concurrency cap.
//! - Transient failures (unreachable, timeout) are retried within a check a
//!   bounded number of times with exponential backoff and jitter. Auth failures
//!   are never retried in a tight loop (camera lockout, SPEC §10.4).
//! - After `failure_threshold` consecutive failures a camera is offline and is
//!   then checked less often: the interval doubles per further failure up to
//!   `offline_max_interval`.
//! - Settings are admin-editable at runtime ([`crate::settings`]) and applied on
//!   the next tick without a restart.
//! - State transitions (not every poll) are persisted and published.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::sync::{watch, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::cameras::{self, CameraState, CameraStatusView, MonitorTarget};
use crate::config::Mode;
use crate::credentials::CredentialStore;
use crate::events::{Event, EventBus};
use crate::providers::{CameraProvider, ErrorCategory, ProviderError, ProviderFactory};
use crate::settings::{MonitoringSettings, RuntimeSettings};

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
    /// Extra attempts within one check for transient errors.
    pub retries: u32,
    /// Upper bound for an offline camera's backed-off interval.
    pub offline_max_interval: Duration,
    /// How often an online camera's identity and credentials are re-checked.
    pub identity_refresh: Duration,
}

impl From<&MonitoringSettings> for PollingPolicy {
    fn from(c: &MonitoringSettings) -> Self {
        Self {
            interval: Duration::from_secs(c.interval_secs),
            jitter: f64::from(c.jitter_percent) / 100.0,
            failure_threshold: c.failure_threshold,
            timeout: Duration::from_secs(c.timeout_secs),
            max_in_flight: usize::try_from(c.max_concurrent).unwrap_or(1).max(1),
            retries: c.retries,
            offline_max_interval: Duration::from_secs(c.offline_max_interval_secs),
            identity_refresh: Duration::from_secs(c.identity_refresh_minutes * 60),
        }
    }
}

/// Delay before retry `attempt` (0-based) within one check: 250 ms doubling,
/// ±jitter, capped at 2 s.
const RETRY_BASE: Duration = Duration::from_millis(250);

impl PollingPolicy {
    fn jittered(&self, base: Duration, unit: f64) -> Duration {
        let spread = (unit.clamp(0.0, 1.0) * 2.0 - 1.0) * self.jitter;
        base.mul_f64((1.0 + spread).max(0.0))
    }

    /// `interval` randomized by ±`jitter`, using `unit` in [0, 1).
    pub fn next_delay(&self, unit: f64) -> Duration {
        self.jittered(self.interval, unit)
    }

    /// When to check again after a check that produced `status`.
    pub fn delay_after(&self, status: &CameraStatusView, unit: f64) -> Duration {
        // The status call itself was rejected: back off fully so a wrong
        // password can't lock out the camera account.
        if status.error_category == Some(ErrorCategory::AuthFailed) {
            return self.jittered(self.offline_max_interval, unit);
        }
        let threshold = self.failure_threshold.max(1);
        if status.consecutive_failures >= threshold {
            let extra = (status.consecutive_failures - threshold + 1).min(16);
            let backed = self.interval.saturating_mul(1 << extra);
            return self.jittered(backed.min(self.offline_max_interval), unit);
        }
        self.next_delay(unit)
    }

    pub fn retry_delay(&self, attempt: u32, unit: f64) -> Duration {
        let base = RETRY_BASE
            .saturating_mul(1 << attempt.min(3))
            .min(Duration::from_secs(2));
        self.jittered(base, unit)
    }
}

/// Counters for the polling service (SPEC §10.3: a supervised service with its
/// own metrics). Shared across restarts of the monitor.
#[derive(Debug, Default)]
pub struct MonitorStats {
    checks: AtomicU64,
    failures: AtomicU64,
    retries: AtomicU64,
    identity_checks: AtomicU64,
    in_flight: AtomicUsize,
    cameras: AtomicUsize,
    last_tick_unix: AtomicU64,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct MonitorStatsSnapshot {
    /// Status checks completed since the server started.
    pub checks_total: u64,
    /// Checks that ended in a failure (after retries).
    pub failures_total: u64,
    /// Extra attempts made for transient errors.
    pub retries_total: u64,
    pub identity_checks_total: u64,
    pub checks_in_flight: usize,
    pub cameras_monitored: usize,
    /// Unix time of the monitor's last scheduling tick (0 = never).
    pub last_tick_unix: u64,
}

impl MonitorStats {
    pub fn snapshot(&self) -> MonitorStatsSnapshot {
        MonitorStatsSnapshot {
            checks_total: self.checks.load(Ordering::Relaxed),
            failures_total: self.failures.load(Ordering::Relaxed),
            retries_total: self.retries.load(Ordering::Relaxed),
            identity_checks_total: self.identity_checks.load(Ordering::Relaxed),
            checks_in_flight: self.in_flight.load(Ordering::Relaxed),
            cameras_monitored: self.cameras.load(Ordering::Relaxed),
            last_tick_unix: self.last_tick_unix.load(Ordering::Relaxed),
        }
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

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Fetch identity, capabilities and stream profiles and store them. Unsupported
/// capability or stream APIs keep whatever was stored before. The outcome is
/// recorded as the camera's identity status, so wrong credentials show up even
/// while the camera is reachable.
pub async fn refresh_identity(
    db: &SqlitePool,
    provider: &dyn CameraProvider,
    id: Uuid,
    timeout: Duration,
) -> Result<(), RefreshError> {
    let result = fetch_identity(db, provider, id, timeout).await;
    match &result {
        Ok(()) => cameras::record_identity_result(db, id, None).await?,
        // Reachability problems belong to the status check, not identity.
        Err(RefreshError::Provider(e))
            if !matches!(
                e.category(),
                ErrorCategory::Unreachable | ErrorCategory::Timeout
            ) =>
        {
            cameras::record_identity_result(db, id, Some(e)).await?;
        }
        Err(_) => {}
    }
    result
}

async fn fetch_identity(
    db: &SqlitePool,
    provider: &dyn CameraProvider,
    id: Uuid,
    timeout: Duration,
) -> Result<(), RefreshError> {
    let bounded = |what: &'static str| {
        ProviderError::Timeout(format!("{what} took longer than {}s", timeout.as_secs()))
    };
    let device = tokio::time::timeout(timeout, provider.get_device_info())
        .await
        .map_err(|_| bounded("device info"))??;
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
    cameras::save_identity(db, id, &device, caps.as_ref(), profiles.as_deref()).await?;
    Ok(())
}

struct Track {
    provider: Arc<dyn CameraProvider>,
    /// Camera config, shared credentials and settings generation; a change
    /// rebuilds the provider.
    version: (OffsetDateTime, Option<OffsetDateTime>, u64),
    next_due: Arc<Mutex<Instant>>,
    in_flight: Arc<AtomicBool>,
    /// When identity/credentials were last checked by this monitor.
    last_identity: Arc<Mutex<Option<Instant>>>,
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

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Monitor {
    db: SqlitePool,
    mode: Mode,
    factory: ProviderFactory,
    bus: EventBus,
    settings: watch::Receiver<RuntimeSettings>,
    policy: PollingPolicy,
    generation: u64,
    tracks: HashMap<Uuid, Track>,
    permits: Arc<Semaphore>,
    jitter: Arc<Jitter>,
    store: Arc<dyn CredentialStore>,
    stats: Arc<MonitorStats>,
    /// How often the registry is re-read to pick up added/removed cameras.
    tick: Duration,
}

impl Monitor {
    pub fn new(
        db: SqlitePool,
        mode: Mode,
        bus: EventBus,
        mut settings: watch::Receiver<RuntimeSettings>,
        store: Arc<dyn CredentialStore>,
        stats: Arc<MonitorStats>,
    ) -> Self {
        let policy = PollingPolicy::from(&settings.borrow_and_update().monitoring);
        Self {
            db,
            mode,
            factory: ProviderFactory::new(mode, policy.timeout),
            bus,
            settings,
            permits: Arc::new(Semaphore::new(policy.max_in_flight)),
            policy,
            generation: 0,
            tracks: HashMap::new(),
            jitter: Arc::new(Jitter::new()),
            store,
            stats,
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
            self.apply_settings_changes();
            if let Err(err) = self.tick_once(&cancel).await {
                tracing::warn!(error = %err, "status monitor could not read cameras");
            }
        }
        tracing::info!("camera status monitor stopped");
    }

    /// Pick up admin changes to the monitoring settings.
    fn apply_settings_changes(&mut self) {
        if !self.settings.has_changed().unwrap_or(false) {
            return;
        }
        let policy = PollingPolicy::from(&self.settings.borrow_and_update().monitoring);
        if policy.max_in_flight != self.policy.max_in_flight {
            // In-flight checks keep their permits from the old semaphore.
            self.permits = Arc::new(Semaphore::new(policy.max_in_flight));
        }
        if policy.timeout != self.policy.timeout {
            self.factory = ProviderFactory::new(self.mode, policy.timeout);
            self.generation += 1; // rebuild providers with the new timeout
        }
        if policy.interval < self.policy.interval {
            // Pull already-scheduled checks forward so a shorter interval takes
            // effect now, not after the old (longer) interval runs out.
            let latest = Instant::now() + policy.interval;
            for track in self.tracks.values() {
                let mut due = lock(&track.next_due);
                if *due > latest {
                    *due = Instant::now() + policy.next_delay(self.jitter.unit());
                }
            }
        }
        tracing::info!(
            interval_secs = policy.interval.as_secs(),
            max_in_flight = policy.max_in_flight,
            "monitoring settings changed"
        );
        self.policy = policy;
    }

    async fn tick_once(&mut self, cancel: &CancellationToken) -> Result<(), sqlx::Error> {
        let targets = cameras::monitor_targets(&self.db).await?;
        let now = Instant::now();
        self.stats.cameras.store(targets.len(), Ordering::Relaxed);
        self.stats.last_tick_unix.store(
            u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0),
            Ordering::Relaxed,
        );
        self.tracks
            .retain(|id, _| targets.iter().any(|t| t.id == *id));

        for target in targets {
            let (updated, creds) = target.config_version();
            let version = (updated, creds, self.generation);
            let track = match self.tracks.get_mut(&target.id) {
                Some(track) if track.version == version => track,
                existing => {
                    let provider_target = match target.provider_target(&*self.store) {
                        Ok(t) => t,
                        Err(err) => {
                            tracing::error!(
                                camera_id = %target.id,
                                error = %err,
                                "cannot decrypt camera credentials; skipping camera"
                            );
                            continue;
                        }
                    };
                    // New camera: spread first checks so a big fleet doesn't start
                    // in lockstep. Reconfigured camera: keep its schedule.
                    let next_due = match existing {
                        Some(t) => Arc::clone(&t.next_due),
                        None => {
                            let first = self.policy.interval.mul_f64(self.jitter.unit());
                            Arc::new(Mutex::new(now + first.min(Duration::from_secs(2))))
                        }
                    };
                    self.tracks.insert(
                        target.id,
                        Track {
                            provider: self.factory.create(&provider_target),
                            version,
                            next_due,
                            in_flight: Arc::new(AtomicBool::new(false)),
                            last_identity: Arc::default(),
                        },
                    );
                    self.tracks
                        .get_mut(&target.id)
                        .unwrap_or_else(|| unreachable!("just inserted"))
                }
            };

            if *lock(&track.next_due) > now || track.in_flight.load(Ordering::Acquire) {
                continue;
            }
            let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
                break; // at capacity; remaining cameras wait for the next tick
            };
            track.in_flight.store(true, Ordering::Release);
            // Provisional; the check sets the real next time from its outcome.
            *lock(&track.next_due) = now + self.policy.interval;

            let ctx = CheckContext {
                db: self.db.clone(),
                bus: self.bus.clone(),
                policy: self.policy.clone(),
                provider: Arc::clone(&track.provider),
                in_flight: Arc::clone(&track.in_flight),
                next_due: Arc::clone(&track.next_due),
                last_identity: Arc::clone(&track.last_identity),
                jitter: Arc::clone(&self.jitter),
                stats: Arc::clone(&self.stats),
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
    next_due: Arc<Mutex<Instant>>,
    last_identity: Arc<Mutex<Option<Instant>>>,
    jitter: Arc<Jitter>,
    stats: Arc<MonitorStats>,
}

impl CheckContext {
    /// Whether identity and credentials should be (re)checked now: on first
    /// contact, when the camera comes back online (firmware may have changed),
    /// and periodically so changed or revoked credentials are noticed.
    fn identity_due(&self, target: &MonitorTarget, became_online: bool) -> bool {
        let last = *lock(&self.last_identity);
        became_online
            || target.identity_updated_at.is_none()
            || last.is_none_or(|t| t.elapsed() >= self.policy.identity_refresh)
    }

    /// One status request, with bounded retries for transient errors only.
    async fn status_with_retries(&self) -> CheckOutcome {
        let mut attempt = 0;
        loop {
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
            match &outcome {
                CheckOutcome::Failure(e)
                    if e.category().is_retryable() && attempt < self.policy.retries =>
                {
                    self.stats.retries.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(self.policy.retry_delay(attempt, self.jitter.unit())).await;
                    attempt += 1;
                }
                _ => return outcome,
            }
        }
    }

    async fn check(self, target: MonitorTarget) {
        self.stats.in_flight.fetch_add(1, Ordering::Relaxed);
        let outcome = self.status_with_retries().await;
        self.stats.checks.fetch_add(1, Ordering::Relaxed);
        if matches!(&outcome, CheckOutcome::Failure(e) if e.category() != ErrorCategory::Unsupported)
        {
            self.stats.failures.fetch_add(1, Ordering::Relaxed);
        }
        let (status, transition) = apply(
            &target.status,
            &outcome,
            OffsetDateTime::now_utc(),
            self.policy.failure_threshold,
        );
        *lock(&self.next_due) =
            Instant::now() + self.policy.delay_after(&status, self.jitter.unit());

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

        let became_online = transition.is_some_and(|(_, to)| to == CameraState::Online);
        let mut published = status;
        if published.state == CameraState::Online && self.identity_due(&target, became_online) {
            *lock(&self.last_identity) = Some(Instant::now());
            self.stats.identity_checks.fetch_add(1, Ordering::Relaxed);
            let result =
                refresh_identity(&self.db, &*self.provider, target.id, self.policy.timeout).await;
            if let Err(err) = &result {
                tracing::debug!(camera_id = %target.id, error = %err, "identity refresh failed");
            }
            if let Ok(Some(cam)) = cameras::get(&self.db, target.id).await {
                published = cam.summary.status.clone();
                if result.is_ok() {
                    self.bus.publish(Event::CameraChanged {
                        camera: cam.summary,
                    });
                }
            }
        }

        self.bus.publish(Event::CameraStatus {
            camera_id: target.id,
            status: published,
        });
        self.stats.in_flight.fetch_sub(1, Ordering::Relaxed);
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
            retries: 1,
            offline_max_interval: Duration::from_secs(60),
            identity_refresh: Duration::from_secs(900),
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

    fn policy() -> PollingPolicy {
        PollingPolicy::from(&MonitoringSettings::from(
            &crate::config::MonitoringConfig::default(),
        ))
    }

    #[test]
    fn offline_cameras_back_off_up_to_the_cap() {
        let p = policy(); // 10 s, threshold 3, cap 60 s
        let mid = 0.5; // no jitter
        let with = |failures| CameraStatusView {
            consecutive_failures: failures,
            ..Default::default()
        };
        assert_eq!(p.delay_after(&with(0), mid), Duration::from_secs(10));
        assert_eq!(p.delay_after(&with(2), mid), Duration::from_secs(10));
        assert_eq!(p.delay_after(&with(3), mid), Duration::from_secs(20));
        assert_eq!(p.delay_after(&with(4), mid), Duration::from_secs(40));
        assert_eq!(p.delay_after(&with(5), mid), Duration::from_secs(60));
        assert_eq!(p.delay_after(&with(500), mid), Duration::from_secs(60));
    }

    #[test]
    fn auth_failures_back_off_fully_at_once() {
        let p = policy();
        let status = CameraStatusView {
            consecutive_failures: 1,
            error_category: Some(ErrorCategory::AuthFailed),
            ..Default::default()
        };
        assert_eq!(p.delay_after(&status, 0.5), Duration::from_secs(60));
    }

    #[test]
    fn retry_delays_grow_and_are_bounded() {
        let p = policy();
        assert_eq!(p.retry_delay(0, 0.5), Duration::from_millis(250));
        assert_eq!(p.retry_delay(1, 0.5), Duration::from_millis(500));
        assert_eq!(p.retry_delay(10, 0.5), Duration::from_secs(2));
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
