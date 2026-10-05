use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use device_agent::{
    daemon::Daemon,
    http::{PairRequest, ServiceClient},
    profile::{self, Profile},
    runner,
};
use runtime_protocol::{Metadata, Request, RequestEnvelope};
use runtime_transport::{Client, read_secret};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(version, about = "Conecte seu computador ao Remote Desktop Controller")]
struct Args {
    #[arg(long, global = true)]
    profile: Option<PathBuf>,
    #[arg(long, global = true)]
    daemon: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Parear o computador; a aprovação acontece na sua conta pelo navegador.
    Pair {
        #[arg(long)]
        server: String,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, default_value = "Meu computador")]
        name: String,
        #[arg(long)]
        allow_write: bool,
        #[arg(long)]
        allow_process: bool,
        #[arg(long)]
        allow_terminal: bool,
        #[arg(long)]
        allow_process_network: bool,
        #[arg(long)]
        allow_process_outside_workspace: bool,
        #[arg(long, hide = true)]
        allow_http_loopback: bool,
    },
    /// Executar o agente com reconexão automática.
    Run,
    /// Mostrar configuração pública e status local, sem credenciais.
    Status,
    /// Encerrar o agente e seu daemon.
    Stop,
    /// Ativar ou remover inicialização automática ao entrar no computador.
    Autostart {
        #[arg(value_parser=["enable","disable"])]
        action: String,
    },
    /// Autorizar um executável e argumentos exatos na política local.
    Approve {
        #[arg(long)]
        program: PathBuf,
        #[arg(long, required = true)]
        trust_unconfined: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Aprovar um shell com acesso aos privilégios, rede e arquivos do usuário.
    TerminalApprove {
        #[arg(long)]
        program: PathBuf,
        #[arg(long, required = true)]
        trust_unconfined: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
}

fn default_profile() -> Result<PathBuf> {
    let root = if cfg!(windows) {
        PathBuf::from(
            std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA indisponível; use --profile")?,
        )
        .join("RemoteDesktopController")
    } else {
        PathBuf::from(std::env::var_os("HOME").context("HOME indisponível; use --profile")?)
            .join(".local/share/remotecontroller")
    };
    Ok(root.join("agent"))
}
fn endpoint(root: &Path) -> String {
    let hash = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
    if cfg!(windows) {
        format!(r"\\.\pipe\rdc-agent-{}", &hash[..24])
    } else {
        let path = root.join("runtime/daemon.sock");
        if path.to_string_lossy().len() < 90 {
            path.to_string_lossy().into_owned()
        } else {
            format!("/tmp/rdc-agent-{}/daemon.sock", &hash[..24])
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let args = Args::parse();
    let root = args
        .profile
        .clone()
        .map(Ok)
        .unwrap_or_else(default_profile)?;
    let credential = root.join("agent.json");
    match args.command {
        Action::Pair {
            server,
            workspace,
            name,
            allow_write,
            allow_process,
            allow_terminal,
            allow_process_network,
            allow_process_outside_workspace,
            allow_http_loopback,
        } => {
            device_agent::validate_server(&server, allow_http_loopback)?;
            ensure!(
                !credential.exists(),
                "Computador já pareado; revogue na conta antes de substituir o perfil"
            );
            let workspace = workspace.canonicalize()?;
            let root = profile::protect_profile(&root, &workspace)?;
            let service = ServiceClient::new(&server, allow_http_loopback, None)?;
            let mut config = Profile {
                server,
                device_id: uuid::Uuid::nil(),
                device_token: String::new(),
                workspace,
                state_dir: root.join("runtime"),
                endpoint: endpoint(&root),
                allow_http_loopback,
                allow_write,
                allow_process,
                allow_terminal,
                allow_process_network,
                allow_process_outside_workspace,
            };
            let daemon = Daemon::ensure(
                &config,
                &device_agent::daemon::executable(args.daemon.as_deref())?,
            )
            .await?;
            let result = async {
                let (workspace_id, tools) = runner::catalog(&daemon.client).await?;
                let pairing = service
                    .pair(&PairRequest {
                        name: &name,
                        workspace_id,
                        tools: &tools,
                    })
                    .await?;
                let verification = url::Url::parse(&pairing.verification_uri)?;
                ensure!(
                    verification.origin()
                        == device_agent::validate_server(&config.server, allow_http_loopback)?
                            .origin(),
                    "Página de aprovação mudou de origem"
                );
                println!(
                    "Abra {} e aprove o código {}",
                    pairing.verification_uri, pairing.pairing_code
                );
                let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
                loop {
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "Pareamento expirou; execute pair novamente"
                    );
                    let approved = service
                        .pairing_status(pairing.pairing_id, &pairing.poll_token)
                        .await?;
                    match approved.status.as_str() {
                        "approved" => {
                            config.device_id =
                                approved.device_id.context("Aprovação sem dispositivo")?;
                            config.device_token =
                                approved.device_token.context("Aprovação sem credencial")?;
                            profile::save(&credential, &config)?;
                            println!(
                                "Pareamento concluído. Execute device-agent run ou ative autostart."
                            );
                            break;
                        }
                        "expired" => anyhow::bail!("Pareamento expirou; inicie novamente"),
                        "pending" => tokio::time::sleep(Duration::from_secs(3)).await,
                        _ => anyhow::bail!("Estado de pareamento inválido"),
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            daemon.stop().await?;
            result?;
        }
        Action::Run => {
            let config = profile::load(&credential)?;
            runtime_transport::private_directory(&root)?;
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(root.join("agent.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock).context("Outro agente usa este perfil")?;
            let stop_file = root.join("stop.request");
            if stop_file.exists() {
                std::fs::remove_file(&stop_file)?;
            }
            let daemon_path = device_agent::daemon::executable(args.daemon.as_deref())?;
            let mut daemon = Daemon::ensure(&config, &daemon_path).await?;
            let daemon_secret = read_secret(&config.state_dir.join("ipc.secret"))?;
            let stop = CancellationToken::new();
            let watcher_stop = stop.clone();
            let watcher = tokio::spawn(async move {
                tokio::select! {
                    _=tokio::signal::ctrl_c()=>{},
                    _=async { loop {if stop_file.exists(){break;} tokio::time::sleep(Duration::from_millis(500)).await;} }=>{},
                }
                watcher_stop.cancel();
            });
            println!("Agente ativo. As permissões locais controlam todas as operações.");
            let result = {
                let relay = runner::run(config.clone(), daemon.client.clone(), root, stop.clone());
                tokio::pin!(relay);
                let mut supervision = tokio::time::interval(Duration::from_secs(2));
                loop {
                    tokio::select! {
                        result = &mut relay => break result,
                        _ = supervision.tick(), if !stop.is_cancelled() => {
                            match daemon.recover_owned(&config, &daemon_path, &daemon_secret).await {
                                Ok(true) => tracing::warn!("Daemon recuperado; consulte operações interrompidas antes de repetir efeitos"),
                                Ok(false) => {},
                                Err(error) => { stop.cancel(); break Err(error); }
                            }
                        }
                    }
                }
            };
            watcher.abort();
            daemon.stop().await?;
            result?;
        }
        Action::Status => {
            let config = profile::load(&credential)?;
            let client = Client::new(
                config.endpoint.clone(),
                read_secret(&config.state_dir.join("ipc.secret"))?,
            );
            let heartbeat = client
                .request(
                    &RequestEnvelope {
                        meta: Metadata::new(None, 1000),
                        payload: Request::Heartbeat,
                    },
                    |_| {},
                )
                .await;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"server":config.server,"device_id":config.device_id,"workspace":config.workspace,"daemon_online":heartbeat.is_ok_and(|v|v.payload.is_ok()),"allow_write":config.allow_write,"allow_process":config.allow_process,"allow_terminal":config.allow_terminal})
                )?
            );
        }
        Action::Stop => {
            let config = profile::load(&credential)?;
            if !root.join("stop.request").exists() {
                profile::write_private(&root.join("stop.request"), b"stop")?;
            }
            let client = Client::new(
                config.endpoint,
                read_secret(&config.state_dir.join("ipc.secret"))?,
            );
            let _ = client
                .request(
                    &RequestEnvelope {
                        meta: Metadata::new(None, 3000),
                        payload: Request::Shutdown,
                    },
                    |_| {},
                )
                .await;
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(root.join("agent.lock"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            while fs2::FileExt::try_lock_exclusive(&lock).is_err() {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "Agente ainda encerrando; consulte logs"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            println!("Agente encerrado.");
        }
        Action::Autostart { action } => {
            device_agent::autostart::configure(&root, action == "enable")?;
            println!("Inicialização automática atualizada.");
        }
        Action::Approve {
            program,
            trust_unconfined,
            args: argv,
        } => {
            ensure!(trust_unconfined, "Autorização local explícita necessária");
            let config = profile::load(&credential)?;
            let mut command = tokio::process::Command::new(device_agent::daemon::executable(
                args.daemon.as_deref(),
            )?);
            command
                .args(["policy", "approve", "--program"])
                .arg(program)
                .arg("--trust-unconfined");
            device_agent::daemon::add_args(&mut command, &config);
            if !argv.is_empty() {
                command.arg("--").args(argv);
            }
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            ensure!(
                command.status().await?.success(),
                "Falha na aprovação local"
            );
            println!("Política atualizada. Reinicie o agente para aplicar.");
        }
        Action::TerminalApprove {
            program,
            trust_unconfined,
            args: argv,
        } => {
            ensure!(trust_unconfined, "Autorização local explícita necessária");
            let mut config = profile::load(&credential)?;
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(root.join("agent.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock)
                .context("Pare o agente antes de aprovar o terminal")?;
            drop(lock);
            let mut command = tokio::process::Command::new(device_agent::daemon::executable(
                args.daemon.as_deref(),
            )?);
            command
                .args(["terminal", "approve", "--program"])
                .arg(program)
                .arg("--trust-unconfined");
            device_agent::daemon::add_args(&mut command, &config);
            if !argv.is_empty() {
                command.arg("--").args(argv);
            }
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            ensure!(
                command.status().await?.success(),
                "Falha na aprovação local do terminal"
            );
            config.allow_terminal = true;
            config.allow_process_network = true;
            config.allow_process_outside_workspace = true;
            profile::update(&credential, &config)?;
            println!("Terminal aprovado com os privilégios do usuário. Execute run para aplicar.");
        }
    }
    Ok(())
}
