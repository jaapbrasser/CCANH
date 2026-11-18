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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: IpAddr,
    pub port: u16,
    /// Serve `/api/openapi.json` and `/api/docs` without authentication.
    /// Auth arrives in Phase 1; until then the docs are always reachable.
    pub public_api_docs: bool,
    /// Graceful-shutdown timeout in seconds.
    pub shutdown_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 8080,
            public_api_docs: false,
            shutdown_timeout_secs: 10,
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

/// Fully merged configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    /// Directory for the SQLite database, logs and keys.
    pub data_dir: PathBuf,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            data_dir: default_data_dir(),
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
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
    fn demo_mode_uses_separate_database() {
        let mut config = Config::default();
        let prod = config.database_path();
        config.mode = Mode::Demo;
        assert_ne!(prod, config.database_path());
        assert!(config.database_path().ends_with("demo.db"));
    }
}
