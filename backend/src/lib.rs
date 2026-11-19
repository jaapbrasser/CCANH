//! CCANH server library. `main.rs` is a thin CLI wrapper around this crate so
//! integration tests can build and drive the app in-process.

pub mod api;
pub mod audit;
pub mod auth;
pub mod cameras;
pub mod config;
pub mod credentials;
pub mod dashboards;
pub mod demo;
pub mod events;
pub mod monitoring;
pub mod organization;
pub mod providers;
pub mod secret;
pub mod settings;
pub mod storage;
pub mod streaming;
pub mod supervisor;
pub mod tasks;
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
use crate::credentials::CredentialStore;
use crate::events::EventBus;
use crate::monitoring::{Monitor, MonitorStats};
use crate::providers::ProviderFactory;
use crate::settings::{RuntimeSettings, SettingsHandle};
use crate::streaming::opener::ProviderOpener;
use crate::streaming::StreamManager;
use crate::supervisor::ServiceHealth;
use crate::tasks::TaskManager;

/// Shared application state, built once at startup and cloned into handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: SqlitePool,
    pub started_at: Instant,
    pub login_limiter: Arc<LoginLimiter>,
    pub setup: Arc<SetupState>,
    pub events: EventBus,
    pub services: ServiceHealth,
    /// Admin-editable runtime settings (defaults from `config`).
    pub settings: SettingsHandle,
    pub monitor_stats: Arc<MonitorStats>,
    pub tasks: TaskManager,
    /// Shared upstream video streams (SPEC §7).
    pub streams: StreamManager,
    /// Encrypts camera credentials at rest (ADR 0003).
    pub credentials: Arc<dyn CredentialStore>,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool, credentials: Arc<dyn CredentialStore>) -> Self {
        let settings = SettingsHandle::new(RuntimeSettings::from_config(&config));
        let events = EventBus::default();
        let streams = StreamManager::new(
            Arc::new(ProviderOpener {
                db: db.clone(),
                store: Arc::clone(&credentials),
                settings: settings.clone(),
                mode: config.mode,
            }),
            settings.clone(),
            events.clone(),
        );
        Self {
            streams,
            tasks: TaskManager::new(db.clone(), events.clone()),
            events,
            services: ServiceHealth::default(),
            settings,
            monitor_stats: Arc::default(),
            credentials,
            config: Arc::new(config),
            db,
            started_at: Instant::now(),
            login_limiter: Arc::new(LoginLimiter::default()),
            setup: Arc::new(SetupState::default()),
        }
    }

    /// A provider factory using the current request timeout setting.
    pub fn providers(&self) -> ProviderFactory {
        let timeout = self.settings.current().monitoring.timeout_secs;
        ProviderFactory::new(self.config.mode, Duration::from_secs(timeout))
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
    let monitor = tokio::spawn(supervisor::supervise(
        "status_monitor",
        state.services.clone(),
        shutdown.child_token(),
        {
            let (db, bus, mode) = (state.db.clone(), state.events.clone(), state.config.mode);
            let (store, stats) = (state.credentials.clone(), state.monitor_stats.clone());
            let settings = state.settings.clone();
            move |cancel| {
                Monitor::new(
                    db.clone(),
                    mode,
                    bus.clone(),
                    settings.subscribe(),
                    store.clone(),
                    stats.clone(),
                )
                .run(cancel)
            }
        },
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

    // Stream metrics once a second while streams exist.
    tokio::spawn(state.streams.clone().run_sampler(shutdown.child_token()));
    // Cancel running tasks and close streams as soon as shutdown starts, so
    // open media sockets don't hold up graceful shutdown.
    {
        let tasks = state.tasks.clone();
        let streams = state.streams.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            shutdown.cancelled().await;
            tasks.shutdown();
            streams.shutdown();
        });
    }
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
    let _ = monitor.await;
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
        match tasks::purge_old(&db).await {
            Ok(0) => {}
            Ok(n) => tracing::debug!(removed = n, "purged old tasks"),
            Err(err) => tracing::warn!(error = %err, "failed to purge old tasks"),
        }
    }
}
