//! `crane` executable: CLI parsing, startup and shutdown.

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use tokio_util::sync::CancellationToken;

use crane::config::{Config, Overrides};
use crane::{storage, telemetry, AppState};

#[derive(Debug, Parser)]
#[command(
    name = "crane",
    version,
    about = "Crane Camera Automated Navigation Hub server"
)]
struct Cli {
    /// Path to the TOML config file (default: config.toml next to the executable).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Data directory for the database, logs and keys.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Address to bind to.
    #[arg(long)]
    bind: Option<IpAddr>,

    /// Port to listen on.
    #[arg(long)]
    port: Option<u16>,

    /// Log filter, e.g. `info` or `crane=debug`. `RUST_LOG` takes precedence.
    #[arg(long, value_name = "FILTER")]
    log_level: Option<String>,

    /// Print the OpenAPI document as JSON to stdout and exit. Needs no config,
    /// database or network.
    #[arg(long)]
    print_openapi: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.print_openapi {
        return match crane::api::openapi().to_pretty_json() {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: failed to serialise OpenAPI document: {err}");
                ExitCode::FAILURE
            }
        };
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("error: failed to start async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Logging may not be initialised yet, so always print to stderr too.
            tracing::error!(error = format!("{err:#}"), "fatal error");
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let overrides = Overrides {
        data_dir: cli.data_dir,
        bind: cli.bind,
        port: cli.port,
        log_level: cli.log_level,
    };
    let config = Config::load(cli.config.as_deref(), &overrides)?;
    let _telemetry = telemetry::init(&config)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = ?config.mode,
        data_dir = %config.data_dir.display(),
        "starting crane"
    );

    if !config.server.bind.is_loopback() {
        // Server-side TLS arrives in Phase 1 (§14.3).
        tracing::warn!(
            bind = %config.server.bind,
            "serving plain HTTP on a non-loopback address; traffic is not encrypted"
        );
    }

    let db_path = config.database_path();
    let db = storage::connect(&db_path)
        .await
        .with_context(|| format!("opening database {}", db_path.display()))?;

    let addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;
    let local = listener.local_addr().unwrap_or(addr);
    tracing::info!(address = %local, "listening on http://{local}");

    let shutdown = CancellationToken::new();
    tokio::spawn(wait_for_signal(shutdown.clone()));

    crane::serve(listener, AppState::new(config, db), shutdown).await?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// Cancel `token` on Ctrl+C (and SIGTERM on Unix).
async fn wait_for_signal(token: CancellationToken) {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
    token.cancel();
}
