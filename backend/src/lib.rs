//! CCANH server library. `main.rs` is a thin CLI wrapper around this crate so
//! integration tests can build and drive the app in-process.

pub mod api;
pub mod config;
pub mod storage;
pub mod telemetry;

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

/// Shared application state, built once at startup and cloned into handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool) -> Self {
        Self {
            config: Arc::new(config),
            db,
            started_at: Instant::now(),
        }
    }
}

/// Build the HTTP application for the given state.
pub fn app(state: AppState) -> axum::Router {
    api::router(state)
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("server error: {0}")]
    Io(#[from] std::io::Error),
}

/// Serve `state` on `listener` until `shutdown` is cancelled, then wait up to
/// `config.server.shutdown_timeout_secs` for in-flight requests to finish.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: CancellationToken,
) -> Result<(), ServeError> {
    let timeout = Duration::from_secs(state.config.server.shutdown_timeout_secs);
    let db = state.db.clone();
    let signal = shutdown.clone();

    let server = axum::serve(listener, app(state))
        .with_graceful_shutdown(async move { signal.cancelled().await });

    let mut server = std::pin::pin!(server.into_future());
    tokio::select! {
        result = &mut server => result?,
        () = async {
            shutdown.cancelled().await;
            tokio::time::sleep(timeout).await;
        } => {
            tracing::warn!(timeout_secs = timeout.as_secs(), "graceful shutdown timed out; dropping open connections");
        }
    }

    db.close().await;
    Ok(())
}
