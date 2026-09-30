mod host;
mod install;
mod lifecycle;
mod logs;
use anyhow::{Context, Result};
use clap::Parser;
use runtime_domain::ExecutionPolicy;
use runtime_infrastructure::{config::RuntimeConfig, kernel::Kernel, server::serve};
use runtime_transport::{Listener, private_directory, read_secret};
use std::{io::Write, path::PathBuf};
use tokio_util::sync::CancellationToken;
#[derive(Parser)]
#[command(
    version,
    about = "Local-first execution daemon; no public network listener"
)]
struct Args {
    #[command(subcommand)]
    command: Option<lifecycle::Action>,
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    endpoint: Option<String>,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    allow_write: bool,
    #[arg(
        long,
        global = true,
        help = "Trust only the exact program/args rules in config; children are not a filesystem sandbox"
    )]
    allow_process: bool,
    #[arg(
        long,
        global = true,
        help = "Allow trusted child programs outside workspace; no OS sandbox"
    )]
    allow_process_outside_workspace: bool,
    #[arg(
        long,
        global = true,
        help = "Allow network effects of trusted child programs"
    )]
    allow_process_network: bool,
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut raw = std::env::args_os();
    raw.next();
    #[cfg(windows)]
    if raw.next().as_deref() == Some(std::ffi::OsStr::new("--finish-uninstall")) {
        let profile = PathBuf::from(raw.next().context("uninstall worker profile missing")?);
        install::finish_uninstall(&profile)?;
        return Ok(());
    }
    #[cfg(unix)]
    if raw.next().as_deref() == Some(std::ffi::OsStr::new("--process-guardian")) {
        std::process::exit(runtime_infrastructure::guardian::run(raw.collect()).await?);
    }
    let mut args = Args::parse();
    lifecycle::resolve(&mut args)?;
    if lifecycle::dispatch(&args).await? {
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_writer(logs::Writer::new(
            args.state_dir.as_ref().unwrap().join("daemon.log"),
        )?)
        .with_ansi(false)
        .init();
    private_directory(args.state_dir.as_ref().unwrap()).context("protect state directory")?;
    let state = args.state_dir.as_ref().unwrap().canonicalize()?;
    let root = args.workspace.as_ref().unwrap().canonicalize()?;
    anyhow::ensure!(
        !state.starts_with(&root),
        "state directory must be outside workspace"
    );
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state.join("daemon.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("another daemon owns this state directory")?;
    let token_file = state.join("ipc.secret");
    let secret = if token_file.exists() {
        read_secret(&token_file)?
    } else {
        let mut secret = uuid::Uuid::new_v4().as_bytes().to_vec();
        secret.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        let mut f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&token_file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(&secret)?;
        f.sync_all()?;
        secret
    };
    let endpoint = args
        .endpoint
        .clone()
        .unwrap_or_else(|| default_endpoint(&state));
    let listener = Listener::bind(&endpoint).await?;
    let config: RuntimeConfig = match args.config.as_ref() {
        Some(path) if path.exists() => serde_json::from_slice(&tokio::fs::read(path).await?)?,
        _ => RuntimeConfig::default(),
    };
    #[cfg(unix)]
    let config = {
        let mut config = config;
        config.guardian_path = Some(std::env::current_exe()?);
        config
    };
    let capabilities = lifecycle::capabilities(&args);
    let kernel = Kernel::open(&root, &state, config, ExecutionPolicy::new(capabilities)).await?;
    tracing::info!(endpoint=%endpoint,workspace=%kernel.workspace.id,"daemon ready");
    let stop = CancellationToken::new();
    let signal_stop = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_stop.cancel();
        }
    });
    serve(listener, kernel, secret, stop).await?;
    drop(lock);
    Ok(())
}
fn default_endpoint(state: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(state.to_string_lossy().as_bytes()));
        format!(r"\\.\pipe\local-runtime-{}", &hash[..16])
    }
    #[cfg(unix)]
    {
        state.join("daemon.sock").to_string_lossy().into()
    }
}
