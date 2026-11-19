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
    /// Windows: register crane as a service that starts with Windows, then
    /// exit. Run as administrator; other options given here (data dir, config,
    /// bind, port, demo) become the service's options.
    #[arg(long, conflicts_with_all = ["uninstall_service", "service", "create_admin"])]
    install_service: bool,
    /// Windows: stop and remove the service, then exit. Data is kept.
    #[arg(long, conflicts_with_all = ["service", "create_admin"])]
    uninstall_service: bool,
    /// Run under the Windows service manager (set by --install-service).
    #[arg(long, hide = true)]
    service: bool,
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

    if cli.install_service || cli.uninstall_service || cli.service {
        let result = if cli.install_service {
            service::install(&cli)
        } else if cli.uninstall_service {
            service::uninstall()
        } else {
            service::run()
        };
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:#}");
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

    match runtime.block_on(run_with(cli, CancellationToken::new(), true)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Logging may not be initialised yet, so always print to stderr too.
            tracing::error!(error = format!("{err:#}"), "fatal error");
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Run the server until `shutdown` is cancelled (or, with `listen_for_signals`,
/// until Ctrl+C / SIGTERM).
async fn run_with(
    cli: Cli,
    shutdown: CancellationToken,
    listen_for_signals: bool,
) -> anyhow::Result<()> {
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
    let credentials = crane::credentials::platform_store(&config.data_dir)
        .context("setting up the camera credential store")?;
    tracing::info!(store = credentials.name(), "camera credential store ready");
    if demo {
        let seeded = crane::demo::seed(&db, &*credentials).await?;
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

    let state = AppState::new(config, db, credentials);
    state
        .settings
        .load(&state.db)
        .await
        .context("loading runtime settings")?;
    let interrupted = state.tasks.mark_interrupted().await?;
    if interrupted > 0 {
        tracing::warn!(
            count = interrupted,
            "marked tasks interrupted by the last shutdown as failed"
        );
    }
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

    if listen_for_signals {
        tokio::spawn(wait_for_signal(shutdown.clone()));
    }

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

/// Windows service mode (SPEC §16, ADR 0010): runs as LocalSystem, which
/// matches the DPAPI machine-scope credential store (ADR 0003).
#[cfg(windows)]
mod service {
    use std::ffi::OsString;
    use std::time::Duration;

    use anyhow::Context;
    use clap::Parser;
    use tokio_util::sync::CancellationToken;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{define_windows_service, service_dispatcher};

    use super::{run_with, Cli};

    pub const NAME: &str = "CraneCameraHub";
    const DISPLAY_NAME: &str = "Crane Camera Automated Navigation Hub";
    const DESCRIPTION: &str = "Manages AXIS IP cameras and serves the CCANH web interface.";

    fn absolute(p: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        std::path::absolute(p).with_context(|| format!("resolving {}", p.display()))
    }

    pub fn install(cli: &Cli) -> anyhow::Result<()> {
        let exe = std::env::current_exe().context("finding this executable")?;
        // Services start in System32, so every path is made absolute now.
        let data_dir = match &cli.data_dir {
            Some(d) => absolute(d)?,
            None => crane::config::default_data_dir(),
        };
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("creating data directory {}", data_dir.display()))?;
        let mut args: Vec<OsString> = vec![
            "--service".into(),
            "--data-dir".into(),
            data_dir.clone().into(),
        ];
        if let Some(c) = &cli.config {
            args.extend(["--config".into(), absolute(c)?.into()]);
        }
        if let Some(b) = cli.bind {
            args.extend(["--bind".into(), b.to_string().into()]);
        }
        if let Some(p) = cli.port {
            args.extend(["--port".into(), p.to_string().into()]);
        }
        if let Some(l) = &cli.log_level {
            args.extend(["--log-level".into(), l.into()]);
        }
        if cli.demo {
            args.push("--demo".into());
        }
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .context("connecting to the Windows service manager (run this as administrator)")?;
        let info = ServiceInfo {
            name: NAME.into(),
            display_name: DISPLAY_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: args,
            dependencies: Vec::new(),
            account_name: None, // LocalSystem
            account_password: None,
        };
        let service = manager
            .create_service(&info, ServiceAccess::CHANGE_CONFIG)
            .context("creating the service (is it already installed? run as administrator)")?;
        service
            .set_description(DESCRIPTION)
            .context("setting the service description")?;
        println!(
            "Installed the \"{DISPLAY_NAME}\" service ({NAME}); it starts with Windows.\n\
             Data directory: {}\nStart it now with: sc start {NAME}",
            data_dir.display()
        );
        Ok(())
    }

    pub fn uninstall() -> anyhow::Result<()> {
        let manager =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
                .context("connecting to the Windows service manager (run this as administrator)")?;
        let service = manager
            .open_service(
                NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
            )
            .context("opening the service (is it installed? run as administrator)")?;
        if service.query_status()?.current_state != ServiceState::Stopped {
            // Already stopping is fine; wait below either way.
            let _ = service.stop();
            for _ in 0..120 {
                if service.query_status()?.current_state == ServiceState::Stopped {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        service.delete().context("removing the service")?;
        println!("Removed the {NAME} service. The data directory was kept.");
        Ok(())
    }

    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> anyhow::Result<()> {
        service_dispatcher::start(NAME, ffi_service_main)
            .context("connecting to the service manager; --service is only for use by Windows (see --install-service)")
    }

    fn service_main(_args: Vec<OsString>) {
        if let Err(err) = run_service() {
            tracing::error!(error = format!("{err:#}"), "service stopped with an error");
        }
    }

    fn status(
        state: ServiceState,
        accept: ServiceControlAccept,
        exit: ServiceExitCode,
        wait: Duration,
    ) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: exit,
            checkpoint: 0,
            wait_hint: wait,
            process_id: None,
        }
    }

    fn run_service() -> anyhow::Result<()> {
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let handle = service_control_handler::register(NAME, move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop.cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("registering the service control handler")?;
        let none = ServiceControlAccept::empty();
        handle.set_service_status(status(
            ServiceState::StartPending,
            none,
            ServiceExitCode::NO_ERROR,
            Duration::from_secs(30),
        ))?;
        // The options set at install time are on this process's command line.
        let cli = Cli::parse();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("starting the async runtime")?;
        handle.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ServiceExitCode::NO_ERROR,
            Duration::ZERO,
        ))?;
        let result = runtime.block_on(run_with(cli, shutdown, false));
        let exit = match &result {
            Ok(()) => ServiceExitCode::NO_ERROR,
            Err(_) => ServiceExitCode::ServiceSpecific(1),
        };
        handle.set_service_status(status(ServiceState::Stopped, none, exit, Duration::ZERO))?;
        result
    }
}

#[cfg(not(windows))]
mod service {
    use super::Cli;

    fn windows_only() -> anyhow::Result<()> {
        anyhow::bail!("service mode is only available on Windows; run crane directly (or under systemd/launchd)")
    }

    pub fn install(_: &Cli) -> anyhow::Result<()> {
        windows_only()
    }

    pub fn uninstall() -> anyhow::Result<()> {
        windows_only()
    }

    pub fn run() -> anyhow::Result<()> {
        windows_only()
    }
}
