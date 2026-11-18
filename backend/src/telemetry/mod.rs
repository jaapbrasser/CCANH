//! Tracing setup: console output plus optional rolling daily log files.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::config::{Config, LogFormat};

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("failed to create log directory {path}: {source}")]
    LogDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create log file appender: {0}")]
    Appender(#[from] tracing_appender::rolling::InitError),
    #[error("invalid log filter: {0}")]
    Filter(#[from] tracing_subscriber::filter::ParseError),
    #[error("a global tracing subscriber is already installed")]
    AlreadyInitialized,
}

/// Keeps the background log writer alive. Drop it only at shutdown so buffered
/// log lines are flushed.
#[must_use = "dropping the guard stops file logging"]
pub struct TelemetryGuard {
    _file: Option<WorkerGuard>,
}

/// Build the filter: `RUST_LOG` if set, otherwise the configured level.
fn filter(default: &str) -> Result<EnvFilter, TelemetryError> {
    match std::env::var("RUST_LOG") {
        Ok(value) if !value.trim().is_empty() => Ok(EnvFilter::try_new(value)?),
        _ => Ok(EnvFilter::try_new(default)?),
    }
}

/// Install the global tracing subscriber.
pub fn init(config: &Config) -> Result<TelemetryGuard, TelemetryError> {
    let console = match config.logging.format {
        LogFormat::Pretty => fmt::layer().with_target(true).boxed(),
        LogFormat::Json => fmt::layer().json().with_current_span(true).boxed(),
    };

    let mut guard = None;
    let file_layer = if config.logging.file {
        let dir = config.log_dir();
        std::fs::create_dir_all(&dir).map_err(|source| TelemetryError::LogDir {
            path: dir.clone(),
            source,
        })?;
        let appender = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("crane")
            .filename_suffix("log")
            .max_log_files(config.logging.retention_days)
            .build(&dir)?;
        let (writer, worker_guard) = tracing_appender::non_blocking(appender);
        guard = Some(worker_guard);
        Some(fmt::layer().json().with_ansi(false).with_writer(writer))
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(filter(&config.logging.level)?)
        .with(console)
        .with(file_layer)
        .try_init()
        .map_err(|_| TelemetryError::AlreadyInitialized)?;

    Ok(TelemetryGuard { _file: guard })
}
