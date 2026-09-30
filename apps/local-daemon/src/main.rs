use anyhow::{Context, Result};
use clap::Parser;
use runtime_domain::{Capability, ExecutionPolicy};
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
    #[arg(long)]
    workspace: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    allow_write: bool,
    #[arg(
        long,
        help = "Trust only the exact program/args rules in config; children are not a filesystem sandbox"
    )]
    allow_process: bool,
    #[arg(
        long,
        help = "Allow trusted child programs outside workspace; no OS sandbox"
    )]
    allow_process_outside_workspace: bool,
    #[arg(long, help = "Allow network effects of trusted child programs")]
    allow_process_network: bool,
    #[arg(long, default_value = "info")]
    log_level: String,
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    private_directory(&args.state_dir).context("protect state directory")?;
    let state = args.state_dir.canonicalize()?;
    let root = args.workspace.canonicalize()?;
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
    let endpoint = args.endpoint.unwrap_or_else(|| default_endpoint(&state));
    let listener = Listener::bind(&endpoint).await?;
    let config: RuntimeConfig = match args.config {
        Some(path) => serde_json::from_slice(&tokio::fs::read(path).await?)?,
        None => RuntimeConfig::default(),
    };
    let mut capabilities = vec![Capability::ReadWorkspace, Capability::GitRead];
    if args.allow_write {
        capabilities.push(Capability::WriteWorkspace);
    }
    if args.allow_process {
        capabilities.extend([Capability::SpawnProcess, Capability::KillProcess]);
    }
    if args.allow_process_outside_workspace {
        capabilities.push(Capability::OutsideWorkspaceAccess);
    }
    if args.allow_process_network {
        capabilities.push(Capability::NetworkAccess);
    }
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
