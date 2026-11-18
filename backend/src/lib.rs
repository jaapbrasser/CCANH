//! CCANH server library. `main.rs` is a thin CLI wrapper around this crate so
//! integration tests can build and drive the app in-process.

pub mod api;
pub mod audit;
pub mod auth;
pub mod config;
pub mod secret;
pub mod storage;
pub mod telemetry;
pub mod tls;
pub mod users;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::auth::rate_limit::LoginLimiter;
use crate::auth::session::{self, Timeouts};
use crate::auth::setup::SetupState;
use crate::config::Config;

/// Shared application state, built once at startup and cloned into handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub started_at: Instant,
    pub login_limiter: Arc<LoginLimiter>,
    pub setup: Arc<SetupState>,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool) -> Self {
        Self {
            config: Arc::new(config),
            db,
            started_at: Instant::now(),
            login_limiter: Arc::new(LoginLimiter::default()),
            setup: Arc::new(SetupState::default()),
        }
    }

    /// If no users exist, issue a one-time setup token and return it so the
    /// caller can show it to the operator.
    pub async fn issue_setup_token_if_needed(&self) -> anyhow::Result<Option<String>> {
        if users::count(&self.db).await? > 0 {
            return Ok(None);
        }
        Ok(Some(self.setup.issue()?))
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

/// How often expired sessions are purged.
const SESSION_SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Serve `state` on `listener` (over TLS when `tls` is given) until `shutdown` is
/// cancelled, then wait up to `config.server.shutdown_timeout_secs` for in-flight
/// requests to finish.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    tls: Option<Arc<rustls::ServerConfig>>,
    shutdown: CancellationToken,
) -> Result<(), ServeError> {
    let timeout = Duration::from_secs(state.config.server.shutdown_timeout_secs);
    let db = state.db.clone();

    let sweeper = tokio::spawn(sweep_sessions(
        state.db.clone(),
        Timeouts::from(&state.config.auth),
        shutdown.child_token(),
    ));

    let handle = axum_server::Handle::new();
    tokio::spawn({
        let handle = handle.clone();
        let shutdown = shutdown.clone();
        async move {
            shutdown.cancelled().await;
            handle.graceful_shutdown(Some(timeout));
        }
    });

    let make_service = app(state).into_make_service_with_connect_info::<SocketAddr>();
    let listener = listener.into_std()?;
    match tls {
        Some(config) => {
            let config = axum_server::tls_rustls::RustlsConfig::from_config(config);
            axum_server::from_tcp_rustls(listener, config)?
                .handle(handle)
                .serve(make_service)
                .await?
        }
        None => {
            axum_server::from_tcp(listener)?
                .handle(handle)
                .serve(make_service)
                .await?
        }
    }

    shutdown.cancel();
    let _ = sweeper.await;
    db.close().await;
    Ok(())
}

async fn sweep_sessions(db: SqlitePool, timeouts: Timeouts, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(SESSION_SWEEP_INTERVAL);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            _ = interval.tick() => {}
        }
        match session::purge_expired(&db, timeouts).await {
            Ok(0) => {}
            Ok(n) => tracing::debug!(removed = n, "purged expired sessions"),
            Err(err) => tracing::warn!(error = %err, "failed to purge expired sessions"),
        }
    }
}
