//! Application configuration.
//!
//! Sources, in order of precedence (highest first):
//! 1. CLI flags
//! 2. `CRANE_*` environment variables (nested keys use `__`, e.g. `CRANE_SERVER__PORT`)
//! 3. The TOML config file (default `config.toml` next to the executable)
//! 4. Built-in defaults
//!
//! The merged configuration is validated once at startup ([`Config::validate`]) so
//! mistakes fail fast with a clear message.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};

/// Errors produced while loading or validating configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to load configuration: {0}")]
    Load(#[from] Box<figment::Error>),
    #[error("invalid configuration: {0}")]
    Invalid(String),
    #[error("config file not found: {0}")]
    FileNotFound(PathBuf),
}

/// Run mode. Demo mode arrives in Phase 2; the setting exists now so the config
/// shape is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Production,
    Demo,
}

/// Output format for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable, coloured output.
    #[default]
    Pretty,
    /// One JSON object per line.
    Json,
}

/// How the server terminates TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// Plain HTTP. A warning is logged when bound to a non-loopback address.
    #[default]
    Off,
    /// Use the PEM certificate chain and private key at `cert_path` / `key_path`.
    Files,
    /// Generate a self-signed certificate in `<data_dir>/tls/` on first run and
    /// reuse it afterwards. Its SHA-256 fingerprint is logged at startup.
    SelfSigned,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    pub mode: TlsMode,
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
    /// Extra DNS names / IPs for the self-signed certificate (localhost and the
    /// machine hostname are always included).
    pub self_signed_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: IpAddr,
    pub port: u16,
    /// Serve `/api/openapi.json` and `/api/docs` without authentication.
    /// Otherwise they are admin-only (SPEC §13.1).
    pub public_api_docs: bool,
    /// Graceful-shutdown timeout in seconds.
    pub shutdown_timeout_secs: u64,
    /// Extra origins (e.g. `https://crane.example.local:8443`) accepted by the
    /// CSRF check on state-changing requests. The server's own origin, derived
    /// from the `Host` header, is always accepted.
    pub allowed_origins: Vec<String>,
    pub tls: TlsConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 8080,
            public_api_docs: false,
            shutdown_timeout_secs: 10,
            allowed_origins: Vec::new(),
            tls: TlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// A session expires after this many minutes without a request.
    pub idle_timeout_minutes: u64,
    /// A session expires this many hours after login regardless of activity.
    pub absolute_timeout_hours: u64,
    /// How often an open WebSocket re-validates its session, in seconds.
    pub websocket_recheck_secs: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            idle_timeout_minutes: 8 * 60,
            absolute_timeout_hours: 7 * 24,
            websocket_recheck_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// Default `tracing` filter directive. `RUST_LOG` overrides it.
    pub level: String,
    pub format: LogFormat,
    /// Write rolling daily log files to `<data_dir>/logs`.
    pub file: bool,
    /// Number of daily log files to keep.
    pub retention_days: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            format: LogFormat::Pretty,
            file: true,
            retention_days: 14,
        }
    }
}

/// Camera status monitoring (SPEC §10.2). Becomes admin-editable in Phase 4.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonitoringConfig {
    /// Seconds between checks of each monitored camera.
    pub interval_secs: u64,
    /// Randomize each interval by ± this percentage.
    pub jitter_percent: u8,
    /// Consecutive failed checks before a camera is marked offline.
    pub failure_threshold: u32,
    /// Timeout for a single check, in seconds.
    pub timeout_secs: u64,
    /// Maximum checks in flight across all cameras.
    pub max_concurrent: usize,
    /// Minutes between identity/credential re-checks of an online camera.
    pub identity_refresh_minutes: u64,
    /// Extra attempts within one check for transient errors (unreachable, timeout).
    pub retries: u32,
    /// Upper bound for the backed-off interval of offline cameras, in seconds.
    pub offline_max_interval_secs: u64,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            interval_secs: 10,
            jitter_percent: 20,
            failure_threshold: 3,
            timeout_secs: 5,
            max_concurrent: 32,
            identity_refresh_minutes: 15,
            retries: 1,
            offline_max_interval_secs: 60,
        }
    }
}

/// Live video (SPEC §7). Becomes admin-editable at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamingConfig {
    /// Seconds an unwatched stream stays connected before closing.
    pub grace_period_secs: u64,
    /// GOP cache cap per stream, in MiB.
    pub stream_buffer_mb: u32,
    /// GOP cache cap across all streams, in MiB.
    pub global_buffer_mb: u32,
    /// Stream profile name for grid tiles, used on cameras that have it.
    /// Unset: the smallest H.264 profile.
    pub default_grid_profile: Option<String>,
    /// Stream profile name for the detail view, used on cameras that have it.
    /// Unset: the largest H.264 profile.
    pub default_detail_profile: Option<String>,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            grace_period_secs: 15,
            stream_buffer_mb: 4,
            global_buffer_mb: 256,
            default_grid_profile: None,
            default_detail_profile: None,
        }
    }
}

/// Fully merged configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    /// Directory for the SQLite database, logs and keys.
    pub data_dir: PathBuf,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub monitoring: MonitoringConfig,
    pub streaming: StreamingConfig,
    pub logging: LoggingConfig,
    /// Demo mode only: keep `demo.db` between runs instead of reseeding.
    pub demo_keep_state: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            data_dir: default_data_dir(),
            server: ServerConfig::default(),
            auth: AuthConfig::default(),
            monitoring: MonitoringConfig::default(),
            streaming: StreamingConfig::default(),
            logging: LoggingConfig::default(),
            demo_keep_state: false,
        }
    }
}

/// CLI-level overrides. Only fields that are `Some` override lower layers.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Overrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind: Option<IpAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo_keep_state: Option<bool>,
}

impl Config {
    /// Load configuration from defaults, an optional file, the given environment
    /// prefix and CLI overrides.
    ///
    /// When `file` is `Some`, the file must exist. When it is `None`, `config.toml`
    /// next to the executable is used if present.
    pub fn load(file: Option<&Path>, overrides: &Overrides) -> Result<Self, ConfigError> {
        Self::load_with_env(file, overrides, Env::prefixed("CRANE_").split("__"))
    }

    fn load_with_env(
        file: Option<&Path>,
        overrides: &Overrides,
        env: Env,
    ) -> Result<Self, ConfigError> {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));

        match file {
            Some(path) => {
                if !path.is_file() {
                    return Err(ConfigError::FileNotFound(path.to_path_buf()));
                }
                figment = figment.merge(Toml::file_exact(path));
            }
            None => {
                if let Some(path) = default_config_path().filter(|p| p.is_file()) {
                    figment = figment.merge(Toml::file_exact(path));
                }
            }
        }

        figment = figment.merge(env);

        // CLI overrides, mapped onto the nested config shape.
        if let Some(dir) = &overrides.data_dir {
            figment = figment.merge(Serialized::default("data_dir", dir));
        }
        if let Some(bind) = overrides.bind {
            figment = figment.merge(Serialized::default("server.bind", bind));
        }
        if let Some(port) = overrides.port {
            figment = figment.merge(Serialized::default("server.port", port));
        }
        if let Some(level) = &overrides.log_level {
            figment = figment.merge(Serialized::default("logging.level", level));
        }
        if let Some(mode) = overrides.mode {
            figment = figment.merge(Serialized::default("mode", mode));
        }
        if let Some(keep) = overrides.demo_keep_state {
            figment = figment.merge(Serialized::default("demo_keep_state", keep));
        }

        let config: Config = figment.extract().map_err(Box::new)?;
        config.validate()?;
        Ok(config)
    }

    /// Check invariants that serde can't express.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.data_dir.as_os_str().is_empty() {
            return Err(ConfigError::Invalid("data_dir must not be empty".into()));
        }
        if self.logging.retention_days == 0 {
            return Err(ConfigError::Invalid(
                "logging.retention_days must be at least 1".into(),
            ));
        }
        if self.server.shutdown_timeout_secs == 0 {
            return Err(ConfigError::Invalid(
                "server.shutdown_timeout_secs must be at least 1".into(),
            ));
        }
        if self.auth.idle_timeout_minutes == 0
            || self.auth.absolute_timeout_hours == 0
            || self.auth.websocket_recheck_secs == 0
        {
            return Err(ConfigError::Invalid(
                "auth timeouts must be at least 1".into(),
            ));
        }
        let m = &self.monitoring;
        if m.interval_secs == 0
            || m.timeout_secs == 0
            || m.failure_threshold == 0
            || m.max_concurrent == 0
        {
            return Err(ConfigError::Invalid(
                "monitoring interval, timeout, failure_threshold and max_concurrent must be at least 1".into(),
            ));
        }
        if m.jitter_percent > 50 {
            return Err(ConfigError::Invalid(
                "monitoring.jitter_percent must be at most 50".into(),
            ));
        }
        if self.server.tls.mode == TlsMode::Files
            && (self.server.tls.cert_path.is_none() || self.server.tls.key_path.is_none())
        {
            return Err(ConfigError::Invalid(
                "server.tls.mode = \"files\" requires server.tls.cert_path and server.tls.key_path"
                    .into(),
            ));
        }
        for origin in &self.server.allowed_origins {
            if !(origin.starts_with("http://") || origin.starts_with("https://"))
                || origin.trim_end_matches('/').matches('/').count() != 2
            {
                return Err(ConfigError::Invalid(format!(
                    "server.allowed_origins entry '{origin}' must look like https://host[:port]"
                )));
            }
        }
        if tracing_subscriber::EnvFilter::try_new(&self.logging.level).is_err() {
            return Err(ConfigError::Invalid(format!(
                "logging.level '{}' is not a valid tracing filter",
                self.logging.level
            )));
        }
        Ok(())
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.server.bind, self.server.port)
    }

    pub fn database_path(&self) -> PathBuf {
        let file = match self.mode {
            Mode::Production => "crane.db",
            Mode::Demo => "demo.db",
        };
        self.data_dir.join(file)
    }

    pub fn log_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn tls_enabled(&self) -> bool {
        self.server.tls.mode != TlsMode::Off
    }
}

/// `%ProgramData%\CraneCamera` on Windows, `./data` elsewhere.
pub fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(program_data) = std::env::var_os("ProgramData") {
            return PathBuf::from(program_data).join("CraneCamera");
        }
    }
    PathBuf::from("data")
}

/// `config.toml` next to the running executable.
fn default_config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn no_env() -> Env {
        Env::prefixed("CRANE_TEST_UNUSED_PREFIX_").split("__")
    }

    #[test]
    fn defaults_are_valid() {
        let config = Config::default();
        config.validate().unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.mode, Mode::Production);
    }

    #[test]
    #[allow(clippy::result_large_err)] // figment::Jail's closure signature is fixed by the crate
    fn precedence_cli_over_env_over_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                "[server]\nport = 1000\nbind = \"0.0.0.0\"\n[logging]\nlevel = \"warn\"\n",
            )?;
            jail.set_env("CRANE_SERVER__PORT", "2000");
            jail.set_env("CRANE_LOGGING__LEVEL", "debug");

            let env = Env::prefixed("CRANE_").split("__");
            let overrides = Overrides {
                port: Some(3000),
                ..Default::default()
            };
            let config =
                Config::load_with_env(Some(Path::new("config.toml")), &overrides, env).unwrap();

            assert_eq!(config.server.port, 3000, "CLI wins over env and file");
            assert_eq!(config.logging.level, "debug", "env wins over file");
            assert_eq!(
                config.server.bind,
                IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                "file wins over defaults"
            );
            Ok(())
        });
    }

    #[test]
    fn missing_explicit_file_fails() {
        let err = Config::load_with_env(
            Some(Path::new("definitely/not/here.toml")),
            &Overrides::default(),
            no_env(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::FileNotFound(_)));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "[server]\nprot = 1").unwrap();
        let err =
            Config::load_with_env(Some(file.path()), &Overrides::default(), no_env()).unwrap_err();
        assert!(matches!(err, ConfigError::Load(_)), "got {err:?}");
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut config = Config::default();
        config.logging.retention_days = 0;
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.logging.level = "not a [valid filter".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn tls_files_mode_requires_paths() {
        let mut config = Config::default();
        config.server.tls.mode = TlsMode::Files;
        assert!(config.validate().is_err());
        config.server.tls.cert_path = Some("c.pem".into());
        config.server.tls.key_path = Some("k.pem".into());
        config.validate().unwrap();
    }

    #[test]
    fn allowed_origins_are_validated() {
        let mut config = Config::default();
        config.server.allowed_origins = vec!["https://crane.local:8443".into()];
        config.validate().unwrap();
        config.server.allowed_origins = vec!["crane.local".into()];
        assert!(config.validate().is_err());
        config.server.allowed_origins = vec!["https://crane.local/path".into()];
        assert!(config.validate().is_err());
    }

    #[test]
    fn example_config_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../config.example.toml");
        let config = Config::load_with_env(Some(&path), &Overrides::default(), no_env())
            .unwrap_or_else(|e| panic!("config.example.toml: {e}"));
        let defaults = Config::default();
        assert_eq!(config.server.port, defaults.server.port);
        assert_eq!(
            config.monitoring.interval_secs,
            defaults.monitoring.interval_secs
        );
    }

    #[test]
    fn demo_mode_uses_separate_database() {
        let mut config = Config::default();
        let prod = config.database_path();
        config.mode = Mode::Demo;
        assert_ne!(prod, config.database_path());
        assert!(config.database_path().ends_with("demo.db"));
    }
}
