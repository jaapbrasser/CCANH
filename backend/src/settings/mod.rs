//! Admin-editable runtime settings (SPEC §3.4).
//!
//! The startup config supplies defaults; overrides are stored in the `settings`
//! table and take effect immediately: services subscribe to a
//! [`tokio::sync::watch`] channel and re-read on change.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use time::OffsetDateTime;
use tokio::sync::watch;
use utoipa::ToSchema;

use crate::config::{DiscoveryConfig, MonitoringConfig, RetentionConfig, StreamingConfig};

/// Camera status monitoring (SPEC §10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MonitoringSettings {
    /// Seconds between checks of each monitored camera (2–3600).
    pub interval_secs: u64,
    /// Randomize each interval by ± this percentage (0–50).
    pub jitter_percent: u8,
    /// Consecutive failed checks before a camera is marked offline (1–20).
    pub failure_threshold: u32,
    /// Timeout for one camera request, in seconds (1–60).
    pub timeout_secs: u64,
    /// Maximum checks in flight across all cameras (1–256).
    pub max_concurrent: u32,
    /// Extra attempts within one check for transient errors (0–5). Never used
    /// for authentication failures.
    pub retries: u32,
    /// Upper bound for the backed-off interval of offline cameras, in seconds
    /// (at least `interval_secs`, at most 3600).
    pub offline_max_interval_secs: u64,
    /// Minutes between identity/credential re-checks of an online camera (1–1440).
    pub identity_refresh_minutes: u64,
}

impl From<&MonitoringConfig> for MonitoringSettings {
    fn from(c: &MonitoringConfig) -> Self {
        Self {
            interval_secs: c.interval_secs,
            jitter_percent: c.jitter_percent,
            failure_threshold: c.failure_threshold,
            timeout_secs: c.timeout_secs,
            max_concurrent: u32::try_from(c.max_concurrent).unwrap_or(u32::MAX),
            retries: c.retries,
            offline_max_interval_secs: c.offline_max_interval_secs,
            identity_refresh_minutes: c.identity_refresh_minutes,
        }
    }
}

/// Partial update; omitted fields keep their current value.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct MonitoringPatch {
    pub interval_secs: Option<u64>,
    pub jitter_percent: Option<u8>,
    pub failure_threshold: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub max_concurrent: Option<u32>,
    pub retries: Option<u32>,
    pub offline_max_interval_secs: Option<u64>,
    pub identity_refresh_minutes: Option<u64>,
}

impl MonitoringSettings {
    pub fn patched(&self, p: &MonitoringPatch) -> Self {
        Self {
            interval_secs: p.interval_secs.unwrap_or(self.interval_secs),
            jitter_percent: p.jitter_percent.unwrap_or(self.jitter_percent),
            failure_threshold: p.failure_threshold.unwrap_or(self.failure_threshold),
            timeout_secs: p.timeout_secs.unwrap_or(self.timeout_secs),
            max_concurrent: p.max_concurrent.unwrap_or(self.max_concurrent),
            retries: p.retries.unwrap_or(self.retries),
            offline_max_interval_secs: p
                .offline_max_interval_secs
                .unwrap_or(self.offline_max_interval_secs),
            identity_refresh_minutes: p
                .identity_refresh_minutes
                .unwrap_or(self.identity_refresh_minutes),
        }
    }

    /// Check every bound; returns a user-facing message for the first problem.
    pub fn validate(&self) -> Result<(), String> {
        let range = |name: &str, v: u64, lo: u64, hi: u64| {
            if v < lo || v > hi {
                Err(format!("{name} must be between {lo} and {hi}"))
            } else {
                Ok(())
            }
        };
        range("interval_secs", self.interval_secs, 2, 3600)?;
        range("jitter_percent", u64::from(self.jitter_percent), 0, 50)?;
        range(
            "failure_threshold",
            u64::from(self.failure_threshold),
            1,
            20,
        )?;
        range("timeout_secs", self.timeout_secs, 1, 60)?;
        range("max_concurrent", u64::from(self.max_concurrent), 1, 256)?;
        range("retries", u64::from(self.retries), 0, 5)?;
        range(
            "offline_max_interval_secs",
            self.offline_max_interval_secs,
            self.interval_secs,
            3600,
        )?;
        range(
            "identity_refresh_minutes",
            self.identity_refresh_minutes,
            1,
            1440,
        )?;
        Ok(())
    }
}

/// Live video (SPEC §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StreamingSettings {
    /// Seconds an unwatched stream stays connected before closing (0–300).
    pub grace_period_secs: u64,
    /// GOP cache cap per stream in MiB (1–64).
    pub stream_buffer_mb: u32,
    /// GOP cache cap across all streams in MiB (16–4096, at least the per-stream cap).
    pub global_buffer_mb: u32,
    /// Profile name preferred for grid tiles on cameras that have it (≤ 64 chars).
    #[serde(default)]
    pub default_grid_profile: Option<String>,
    /// Profile name preferred for the detail view on cameras that have it (≤ 64 chars).
    #[serde(default)]
    pub default_detail_profile: Option<String>,
    /// Most camera connections at once; 0 = no limit (0–1000).
    #[serde(default)]
    pub max_upstreams: u32,
    /// Connections kept free for detail views when limited (0–100).
    #[serde(default = "default_detail_reserve")]
    pub detail_reserve: u32,
    /// Refuse new connections while inbound video exceeds this (Mbit/s); 0 = no limit (0–100000).
    #[serde(default)]
    pub max_inbound_mbps: u32,
}

fn default_detail_reserve() -> u32 {
    2
}

impl From<&StreamingConfig> for StreamingSettings {
    fn from(c: &StreamingConfig) -> Self {
        Self {
            grace_period_secs: c.grace_period_secs,
            stream_buffer_mb: c.stream_buffer_mb,
            global_buffer_mb: c.global_buffer_mb,
            default_grid_profile: non_empty(c.default_grid_profile.as_deref()),
            default_detail_profile: non_empty(c.default_detail_profile.as_deref()),
            max_upstreams: c.max_upstreams,
            detail_reserve: c.detail_reserve,
            max_inbound_mbps: c.max_inbound_mbps,
        }
    }
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Omitted fields are unchanged. For the profile names, `""` clears the name.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct StreamingPatch {
    pub grace_period_secs: Option<u64>,
    pub stream_buffer_mb: Option<u32>,
    pub global_buffer_mb: Option<u32>,
    pub default_grid_profile: Option<String>,
    pub default_detail_profile: Option<String>,
    pub max_upstreams: Option<u32>,
    pub detail_reserve: Option<u32>,
    pub max_inbound_mbps: Option<u32>,
}

const MAX_PROFILE_NAME: usize = 64;

impl StreamingSettings {
    pub fn patched(&self, p: &StreamingPatch) -> Self {
        let name = |patch: &Option<String>, current: &Option<String>| match patch {
            Some(v) => non_empty(Some(v)),
            None => current.clone(),
        };
        Self {
            grace_period_secs: p.grace_period_secs.unwrap_or(self.grace_period_secs),
            stream_buffer_mb: p.stream_buffer_mb.unwrap_or(self.stream_buffer_mb),
            global_buffer_mb: p.global_buffer_mb.unwrap_or(self.global_buffer_mb),
            default_grid_profile: name(&p.default_grid_profile, &self.default_grid_profile),
            default_detail_profile: name(&p.default_detail_profile, &self.default_detail_profile),
            max_upstreams: p.max_upstreams.unwrap_or(self.max_upstreams),
            detail_reserve: p.detail_reserve.unwrap_or(self.detail_reserve),
            max_inbound_mbps: p.max_inbound_mbps.unwrap_or(self.max_inbound_mbps),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.grace_period_secs > 300 {
            return Err("grace_period_secs must be between 0 and 300".into());
        }
        if !(1..=64).contains(&self.stream_buffer_mb) {
            return Err("stream_buffer_mb must be between 1 and 64".into());
        }
        if !(16..=4096).contains(&self.global_buffer_mb)
            || self.global_buffer_mb < self.stream_buffer_mb
        {
            return Err(
                "global_buffer_mb must be between 16 and 4096 and at least stream_buffer_mb".into(),
            );
        }
        if self.max_upstreams > 1000 {
            return Err("max_upstreams must be between 0 (no limit) and 1000".into());
        }
        if self.detail_reserve > 100 {
            return Err("detail_reserve must be between 0 and 100".into());
        }
        if self.max_inbound_mbps > 100_000 {
            return Err("max_inbound_mbps must be between 0 (no limit) and 100000".into());
        }
        for (field, v) in [
            ("default_grid_profile", &self.default_grid_profile),
            ("default_detail_profile", &self.default_detail_profile),
        ] {
            if v.as_ref().is_some_and(|v| {
                v.chars().count() > MAX_PROFILE_NAME || v.chars().any(char::is_control)
            }) {
                return Err(format!(
                    "{field} must be at most {MAX_PROFILE_NAME} characters, without control characters"
                ));
            }
        }
        Ok(())
    }
}

/// Camera discovery (SPEC §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DiscoverySettings {
    /// Most addresses per job without explicit confirmation (16–65536).
    pub max_addresses: u32,
    /// Hosts probed at once (1–256).
    pub concurrency: u32,
    /// TCP connect timeout per port in ms (100–10000).
    pub connect_timeout_ms: u64,
    /// Identification request timeout in ms (500–30000).
    pub http_timeout_ms: u64,
}

impl From<&DiscoveryConfig> for DiscoverySettings {
    fn from(c: &DiscoveryConfig) -> Self {
        Self {
            max_addresses: c.max_addresses,
            concurrency: c.concurrency,
            connect_timeout_ms: c.connect_timeout_ms,
            http_timeout_ms: c.http_timeout_ms,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct DiscoveryPatch {
    pub max_addresses: Option<u32>,
    pub concurrency: Option<u32>,
    pub connect_timeout_ms: Option<u64>,
    pub http_timeout_ms: Option<u64>,
}

/// Hard ceiling for one scan, even with confirmation (a /16).
pub const MAX_SCAN_ADDRESSES: u32 = 65_536;

impl DiscoverySettings {
    pub fn patched(&self, p: &DiscoveryPatch) -> Self {
        Self {
            max_addresses: p.max_addresses.unwrap_or(self.max_addresses),
            concurrency: p.concurrency.unwrap_or(self.concurrency),
            connect_timeout_ms: p.connect_timeout_ms.unwrap_or(self.connect_timeout_ms),
            http_timeout_ms: p.http_timeout_ms.unwrap_or(self.http_timeout_ms),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(16..=MAX_SCAN_ADDRESSES).contains(&self.max_addresses) {
            return Err(format!(
                "max_addresses must be between 16 and {MAX_SCAN_ADDRESSES}"
            ));
        }
        if !(1..=256).contains(&self.concurrency) {
            return Err("concurrency must be between 1 and 256".into());
        }
        if !(100..=10_000).contains(&self.connect_timeout_ms) {
            return Err("connect_timeout_ms must be between 100 and 10000".into());
        }
        if !(500..=30_000).contains(&self.http_timeout_ms) {
            return Err("http_timeout_ms must be between 500 and 30000".into());
        }
        Ok(())
    }
}

/// History retention (SPEC §11.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RetentionSettings {
    /// Days of camera status history (1–3650).
    pub status_history_days: u32,
    /// Days of finished tasks, discovery scans included (1–3650).
    pub tasks_days: u32,
    /// Days of audit log; 0 keeps it forever, otherwise 90–3650.
    pub audit_days: u32,
}

impl From<&RetentionConfig> for RetentionSettings {
    fn from(c: &RetentionConfig) -> Self {
        Self {
            status_history_days: c.status_history_days,
            tasks_days: c.tasks_days,
            audit_days: c.audit_days,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct RetentionPatch {
    pub status_history_days: Option<u32>,
    pub tasks_days: Option<u32>,
    pub audit_days: Option<u32>,
}

impl RetentionSettings {
    pub fn patched(&self, p: &RetentionPatch) -> Self {
        Self {
            status_history_days: p.status_history_days.unwrap_or(self.status_history_days),
            tasks_days: p.tasks_days.unwrap_or(self.tasks_days),
            audit_days: p.audit_days.unwrap_or(self.audit_days),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3650).contains(&self.status_history_days) {
            return Err("status_history_days must be between 1 and 3650".into());
        }
        if !(1..=3650).contains(&self.tasks_days) {
            return Err("tasks_days must be between 1 and 3650".into());
        }
        if self.audit_days != 0 && !(90..=3650).contains(&self.audit_days) {
            return Err("audit_days must be 0 (keep forever) or between 90 and 3650".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RuntimeSettings {
    pub monitoring: MonitoringSettings,
    pub streaming: StreamingSettings,
    pub discovery: DiscoverySettings,
    pub retention: RetentionSettings,
}

impl RuntimeSettings {
    pub fn from_config(c: &crate::config::Config) -> Self {
        Self {
            monitoring: MonitoringSettings::from(&c.monitoring),
            streaming: StreamingSettings::from(&c.streaming),
            discovery: DiscoverySettings::from(&c.discovery),
            retention: RetentionSettings::from(&c.retention),
        }
    }
}

const MONITORING_KEY: &str = "monitoring";
const STREAMING_KEY: &str = "streaming";
const DISCOVERY_KEY: &str = "discovery";
const RETENTION_KEY: &str = "retention";

/// Shared handle: current values plus the defaults they override.
#[derive(Debug, Clone)]
pub struct SettingsHandle {
    tx: std::sync::Arc<watch::Sender<RuntimeSettings>>,
    defaults: RuntimeSettings,
}

impl SettingsHandle {
    pub fn new(defaults: RuntimeSettings) -> Self {
        let (tx, _) = watch::channel(defaults.clone());
        Self {
            tx: std::sync::Arc::new(tx),
            defaults,
        }
    }

    pub fn current(&self) -> RuntimeSettings {
        self.tx.borrow().clone()
    }

    pub fn defaults(&self) -> &RuntimeSettings {
        &self.defaults
    }

    pub fn subscribe(&self) -> watch::Receiver<RuntimeSettings> {
        self.tx.subscribe()
    }

    async fn stored(db: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar!("SELECT value FROM settings WHERE key = ?1", key)
            .fetch_optional(db)
            .await
    }

    /// Apply stored overrides. Invalid stored values are logged and ignored.
    pub async fn load(&self, db: &SqlitePool) -> Result<(), sqlx::Error> {
        let monitoring = match Self::stored(db, MONITORING_KEY)
            .await?
            .map(|v| serde_json::from_str::<MonitoringSettings>(&v))
        {
            Some(Ok(m)) if m.validate().is_ok() => m,
            Some(_) => {
                tracing::warn!("ignoring invalid stored monitoring settings; using defaults");
                self.defaults.monitoring.clone()
            }
            None => self.defaults.monitoring.clone(),
        };
        let streaming = match Self::stored(db, STREAMING_KEY)
            .await?
            .map(|v| serde_json::from_str::<StreamingSettings>(&v))
        {
            Some(Ok(s)) if s.validate().is_ok() => s,
            Some(_) => {
                tracing::warn!("ignoring invalid stored streaming settings; using defaults");
                self.defaults.streaming.clone()
            }
            None => self.defaults.streaming.clone(),
        };
        let discovery = match Self::stored(db, DISCOVERY_KEY)
            .await?
            .map(|v| serde_json::from_str::<DiscoverySettings>(&v))
        {
            Some(Ok(s)) if s.validate().is_ok() => s,
            Some(_) => {
                tracing::warn!("ignoring invalid stored discovery settings; using defaults");
                self.defaults.discovery.clone()
            }
            None => self.defaults.discovery.clone(),
        };
        let retention = match Self::stored(db, RETENTION_KEY)
            .await?
            .map(|v| serde_json::from_str::<RetentionSettings>(&v))
        {
            Some(Ok(s)) if s.validate().is_ok() => s,
            Some(_) => {
                tracing::warn!("ignoring invalid stored retention settings; using defaults");
                self.defaults.retention.clone()
            }
            None => self.defaults.retention.clone(),
        };
        self.tx.send_replace(RuntimeSettings {
            monitoring,
            streaming,
            discovery,
            retention,
        });
        Ok(())
    }

    async fn persist(db: &SqlitePool, key: &str, json: &str, by: &str) -> Result<(), sqlx::Error> {
        let now = OffsetDateTime::now_utc();
        sqlx::query!(
            "INSERT INTO settings (key, value, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (key) DO UPDATE SET value = ?2, updated_at = ?3, updated_by = ?4",
            key,
            json,
            now,
            by
        )
        .execute(db)
        .await?;
        Ok(())
    }

    /// Validate, persist and publish new monitoring settings. Returns
    /// `(before, after)`.
    pub async fn update_monitoring(
        &self,
        db: &SqlitePool,
        patch: &MonitoringPatch,
        by: &str,
    ) -> Result<(MonitoringSettings, MonitoringSettings), SettingsError> {
        let before = self.current().monitoring;
        let after = before.patched(patch);
        after.validate().map_err(SettingsError::Invalid)?;
        let json =
            serde_json::to_string(&after).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        Self::persist(db, MONITORING_KEY, &json, by).await?;
        self.tx.send_modify(|s| s.monitoring = after.clone());
        Ok((before, after))
    }

    /// Validate, persist and publish new retention settings.
    pub async fn update_retention(
        &self,
        db: &SqlitePool,
        patch: &RetentionPatch,
        by: &str,
    ) -> Result<(RetentionSettings, RetentionSettings), SettingsError> {
        let before = self.current().retention;
        let after = before.patched(patch);
        after.validate().map_err(SettingsError::Invalid)?;
        let json =
            serde_json::to_string(&after).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        Self::persist(db, RETENTION_KEY, &json, by).await?;
        self.tx.send_modify(|s| s.retention = after.clone());
        Ok((before, after))
    }

    pub async fn reset_retention(
        &self,
        db: &SqlitePool,
    ) -> Result<RetentionSettings, SettingsError> {
        sqlx::query!("DELETE FROM settings WHERE key = ?1", RETENTION_KEY)
            .execute(db)
            .await?;
        let defaults = self.defaults.retention.clone();
        self.tx.send_modify(|s| s.retention = defaults.clone());
        Ok(defaults)
    }

    /// Validate, persist and publish new discovery settings.
    pub async fn update_discovery(
        &self,
        db: &SqlitePool,
        patch: &DiscoveryPatch,
        by: &str,
    ) -> Result<(DiscoverySettings, DiscoverySettings), SettingsError> {
        let before = self.current().discovery;
        let after = before.patched(patch);
        after.validate().map_err(SettingsError::Invalid)?;
        let json =
            serde_json::to_string(&after).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        Self::persist(db, DISCOVERY_KEY, &json, by).await?;
        self.tx.send_modify(|s| s.discovery = after.clone());
        Ok((before, after))
    }

    pub async fn reset_discovery(
        &self,
        db: &SqlitePool,
    ) -> Result<DiscoverySettings, SettingsError> {
        sqlx::query!("DELETE FROM settings WHERE key = ?1", DISCOVERY_KEY)
            .execute(db)
            .await?;
        let defaults = self.defaults.discovery.clone();
        self.tx.send_modify(|s| s.discovery = defaults.clone());
        Ok(defaults)
    }

    /// Validate, persist and publish new streaming settings.
    pub async fn update_streaming(
        &self,
        db: &SqlitePool,
        patch: &StreamingPatch,
        by: &str,
    ) -> Result<(StreamingSettings, StreamingSettings), SettingsError> {
        let before = self.current().streaming;
        let after = before.patched(patch);
        after.validate().map_err(SettingsError::Invalid)?;
        let json =
            serde_json::to_string(&after).map_err(|e| SettingsError::Invalid(e.to_string()))?;
        Self::persist(db, STREAMING_KEY, &json, by).await?;
        self.tx.send_modify(|s| s.streaming = after.clone());
        Ok((before, after))
    }

    pub async fn reset_streaming(
        &self,
        db: &SqlitePool,
    ) -> Result<StreamingSettings, SettingsError> {
        sqlx::query!("DELETE FROM settings WHERE key = ?1", STREAMING_KEY)
            .execute(db)
            .await?;
        let defaults = self.defaults.streaming.clone();
        self.tx.send_modify(|s| s.streaming = defaults.clone());
        Ok(defaults)
    }

    /// Remove overrides and go back to the startup defaults.
    pub async fn reset_monitoring(
        &self,
        db: &SqlitePool,
    ) -> Result<MonitoringSettings, SettingsError> {
        sqlx::query!("DELETE FROM settings WHERE key = ?1", MONITORING_KEY)
            .execute(db)
            .await?;
        let defaults = self.defaults.monitoring.clone();
        self.tx.send_modify(|s| s.monitoring = defaults.clone());
        Ok(defaults)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> RuntimeSettings {
        RuntimeSettings::from_config(&crate::config::Config::default())
    }

    #[tokio::test]
    async fn streaming_settings_validate_and_persist() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let handle = SettingsHandle::new(defaults());
        assert!(handle
            .update_streaming(
                &db,
                &StreamingPatch {
                    grace_period_secs: Some(301),
                    ..Default::default()
                },
                "a"
            )
            .await
            .is_err());
        assert!(handle
            .update_streaming(
                &db,
                &StreamingPatch {
                    stream_buffer_mb: Some(64),
                    global_buffer_mb: Some(32),
                    ..Default::default()
                },
                "a"
            )
            .await
            .is_err());
        handle
            .update_streaming(
                &db,
                &StreamingPatch {
                    grace_period_secs: Some(0),
                    ..Default::default()
                },
                "a",
            )
            .await
            .unwrap();
        let fresh = SettingsHandle::new(defaults());
        fresh.load(&db).await.unwrap();
        assert_eq!(fresh.current().streaming.grace_period_secs, 0);
        assert_eq!(
            fresh.current().monitoring,
            defaults().monitoring,
            "groups are independent"
        );
    }

    #[test]
    fn profile_names_are_set_cleared_and_optional_in_stored_json() {
        let d = defaults().streaming;
        let set = d.patched(&StreamingPatch {
            default_grid_profile: Some(" grid ".into()),
            ..Default::default()
        });
        assert_eq!(set.default_grid_profile.as_deref(), Some("grid"));
        assert_eq!(set.default_detail_profile, None);
        let kept = set.patched(&StreamingPatch::default());
        assert_eq!(kept.default_grid_profile.as_deref(), Some("grid"));
        let cleared = set.patched(&StreamingPatch {
            default_grid_profile: Some(String::new()),
            ..Default::default()
        });
        assert_eq!(cleared.default_grid_profile, None);
        assert!(d
            .patched(&StreamingPatch {
                default_detail_profile: Some("x".repeat(65)),
                ..Default::default()
            })
            .validate()
            .is_err());
        // Settings stored before Phase 5b have no profile names.
        let old: StreamingSettings = serde_json::from_str(
            r#"{"grace_period_secs":15,"stream_buffer_mb":4,"global_buffer_mb":256}"#,
        )
        .unwrap();
        assert_eq!(old.default_grid_profile, None);
    }

    #[test]
    fn defaults_are_valid_and_bounds_are_enforced() {
        let d = defaults().monitoring;
        d.validate().unwrap();
        let bad = |p: MonitoringPatch| d.patched(&p).validate().is_err();
        assert!(bad(MonitoringPatch {
            interval_secs: Some(1),
            ..Default::default()
        }));
        assert!(bad(MonitoringPatch {
            jitter_percent: Some(60),
            ..Default::default()
        }));
        assert!(bad(MonitoringPatch {
            max_concurrent: Some(0),
            ..Default::default()
        }));
        assert!(bad(MonitoringPatch {
            retries: Some(9),
            ..Default::default()
        }));
        // Offline cap can't be below the normal interval.
        assert!(bad(MonitoringPatch {
            interval_secs: Some(120),
            offline_max_interval_secs: Some(60),
            ..Default::default()
        }));
    }

    #[tokio::test]
    async fn update_persists_publishes_and_reloads() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let handle = SettingsHandle::new(defaults());
        let mut rx = handle.subscribe();
        let (before, after) = handle
            .update_monitoring(
                &db,
                &MonitoringPatch {
                    interval_secs: Some(30),
                    ..Default::default()
                },
                "admin",
            )
            .await
            .unwrap();
        assert_eq!(before.interval_secs, 10);
        assert_eq!(after.interval_secs, 30);
        assert!(rx.has_changed().unwrap());
        assert_eq!(rx.borrow_and_update().monitoring.interval_secs, 30);

        // A fresh handle (e.g. after restart) picks up the stored override.
        let fresh = SettingsHandle::new(defaults());
        fresh.load(&db).await.unwrap();
        assert_eq!(fresh.current().monitoring.interval_secs, 30);

        let reset = fresh.reset_monitoring(&db).await.unwrap();
        assert_eq!(reset.interval_secs, 10);
        let again = SettingsHandle::new(defaults());
        again.load(&db).await.unwrap();
        assert_eq!(again.current().monitoring.interval_secs, 10);
    }

    #[tokio::test]
    async fn invalid_update_changes_nothing() {
        let db = crate::storage::connect_in_memory().await.unwrap();
        let handle = SettingsHandle::new(defaults());
        let err = handle
            .update_monitoring(
                &db,
                &MonitoringPatch {
                    timeout_secs: Some(0),
                    ..Default::default()
                },
                "admin",
            )
            .await;
        assert!(matches!(err, Err(SettingsError::Invalid(m)) if m.contains("timeout_secs")));
        assert_eq!(handle.current(), defaults());
    }
}
