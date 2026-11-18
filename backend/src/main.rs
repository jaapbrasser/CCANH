//! `crane` executable: CLI parsing, startup and shutdown.

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use tokio_util::sync::CancellationToken;

use crane::config::{Config, Mode, Overrides};
use crane::secret::{ExposeSecret, SecretString};
use crane::{storage, telemetry, tls, AppState};

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

    /// Run in demo mode: simulated cameras, a separate demo.db, no network access
    /// to cameras. Equivalent to `mode = "demo"` in the config file.
    #[arg(long)]
    demo: bool,

    /// Demo mode only: keep demo.db between runs instead of reseeding it.
    #[arg(long, requires = "demo")]
    demo_keep_state: bool,

    /// Create an admin account with this username, prompting for the password,
    /// then exit. Also the recovery path if every admin password is lost.
    #[arg(long, value_name = "USERNAME")]
    create_admin: Option<String>,
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
        mode: cli.demo.then_some(Mode::Demo),
        demo_keep_state: cli.demo_keep_state.then_some(true),
    };
    let config = Config::load(cli.config.as_deref(), &overrides)?;

    if let Some(username) = cli.create_admin {
        return create_admin_interactive(&config, &username).await;
    }

    let _telemetry = telemetry::init(&config)?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = ?config.mode,
        data_dir = %config.data_dir.display(),
        "starting crane"
    );

    let tls = tls::load(&config).context("setting up TLS")?;
    match &tls {
        Some(setup) => tracing::info!(
            certificate = %setup.cert_path.display(),
            sha256 = %setup.fingerprint_sha256,
            generated = setup.generated,
            "TLS enabled; verify this fingerprint when the browser asks"
        ),
        None if !config.server.bind.is_loopback() => tracing::warn!(
            bind = %config.server.bind,
            "serving plain HTTP on a non-loopback address; traffic (including passwords) \
             is not encrypted. Set server.tls.mode to \"files\" or \"self_signed\""
        ),
        None => {}
    }

    let db_path = config.database_path();
    let demo = config.mode == Mode::Demo;
    if demo && !config.demo_keep_state {
        crane::demo::reset_database_file(&db_path)?;
    }
    let db = storage::connect(&db_path)
        .await
        .with_context(|| format!("opening database {}", db_path.display()))?;
    if demo {
        let seeded = crane::demo::seed(&db).await?;
        tracing::warn!(
            database = %db_path.display(),
            seeded_users = seeded.users,
            seeded_cameras = seeded.cameras,
            "DEMO MODE: simulated cameras only; demo accounts are admin/crane-demo-admin \
             and viewer/crane-demo-viewer"
        );
    }

    let addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding to {addr}"))?;
    let local = listener.local_addr().unwrap_or(addr);
    let scheme = if tls.is_some() { "https" } else { "http" };
    tracing::info!(address = %local, "listening on {scheme}://{local}");

    let state = AppState::new(config, db);
    if let Some(token) = state.issue_setup_token_if_needed().await? {
        // SPEC §14.2: the one-time token goes to the console and the log.
        tracing::warn!(
            setup_token = %token,
            "no users exist: open {scheme}://{local}/setup and enter this one-time setup token, \
             or stop the server and run `crane --create-admin <username>`"
        );
        eprintln!(
            "\n  First-run setup: open {scheme}://{local}/setup\n  One-time setup token: {token}\n"
        );
    }

    let shutdown = CancellationToken::new();
    tokio::spawn(wait_for_signal(shutdown.clone()));

    crane::serve(listener, state, tls.map(|t| t.config), shutdown).await?;
    tracing::info!("shutdown complete");
    Ok(())
}

/// `--create-admin`: prompt for a password twice and create the account.
async fn create_admin_interactive(config: &Config, username: &str) -> anyhow::Result<()> {
    let db_path = config.database_path();
    let db = storage::connect(&db_path)
        .await
        .with_context(|| format!("opening database {}", db_path.display()))?;

    let password = SecretString::from(
        rpassword::prompt_password(format!("Password for new admin '{username}': "))
            .context("reading password")?,
    );
    let confirm = SecretString::from(
        rpassword::prompt_password("Repeat password: ").context("reading password")?,
    );
    if password.expose_secret() != confirm.expose_secret() {
        anyhow::bail!("passwords don't match");
    }

    let user = crane::auth::setup::create_admin(&db, username, password).await?;
    db.close().await;
    println!("Created admin '{}' ({}).", user.username, user.id);
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
