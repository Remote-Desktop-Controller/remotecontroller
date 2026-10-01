use crate::Args;
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use runtime_domain::Capability;
use runtime_infrastructure::{
    config::RuntimeConfig,
    maintenance::{Maintenance, RetentionPolicy},
    processes::ProgramRule,
    storage::LocalStore,
};
use runtime_protocol::{Metadata, Request, RequestEnvelope};
use runtime_transport::{Client, private_directory, read_secret};
use sha2::{Digest, Sha256};
use std::{
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
#[derive(Subcommand)]
pub enum Action {
    Serve,
    Init,
    Connect,
    Doctor,
    Stop,
    Install {
        #[arg(long)]
        source: PathBuf,
    },
    Update {
        #[arg(long)]
        source: PathBuf,
    },
    Uninstall,
    RegisterHost {
        #[arg(long)]
        host: String,
        #[arg(long)]
        config_file: PathBuf,
    },
    Policy {
        #[command(subcommand)]
        action: Policy,
    },
    Backup {
        destination: PathBuf,
    },
    Maintain {
        #[arg(long)]
        older_than_ms: Option<u64>,
        #[arg(long, default_value_t = 100)]
        max_records: usize,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        backup: Option<PathBuf>,
    },
}
#[derive(Subcommand)]
pub enum Policy {
    List,
    Revoke {
        #[arg(long)]
        program: PathBuf,
        #[arg(last = true)]
        args: Vec<String>,
    },
    Approve {
        #[arg(long)]
        program: PathBuf,
        #[arg(
            long,
            required = true,
            help = "Explicitly grant unconfined executable access; no OS sandbox"
        )]
        trust_unconfined: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
}
pub fn resolve(args: &mut Args) -> Result<()> {
    let profile = match &args.profile {
        Some(p) => p.clone(),
        None if args.state_dir.is_some() => args.state_dir.clone().unwrap(),
        None => {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .context("profile home unavailable; specify --profile")?;
            PathBuf::from(home).join(".local-runtime")
        }
    };
    private_directory(&profile)?;
    args.profile = Some(profile.canonicalize()?);
    if args.workspace.is_none() {
        args.workspace = Some(std::env::current_dir()?);
    }
    let lexical_root = lexical_absolute(args.workspace.as_ref().unwrap())?;
    let root = args.workspace.as_ref().unwrap().canonicalize()?;
    args.workspace = Some(root.clone());
    if args.state_dir.is_none() {
        let key = format!("{:x}", Sha256::digest(root.to_string_lossy().as_bytes()));
        args.state_dir = Some(
            args.profile
                .as_ref()
                .unwrap()
                .join("states")
                .join(&key[..24]),
        );
    }
    private_directory(args.state_dir.as_ref().unwrap())?;
    ensure!(
        !args
            .state_dir
            .as_ref()
            .unwrap()
            .canonicalize()?
            .starts_with(&root),
        "state must be outside workspace"
    );
    if args.config.is_none() {
        args.config = Some(args.state_dir.as_ref().unwrap().join("runtime.json"));
    }
    let config = args.config.as_ref().unwrap();
    let lexical_config = lexical_absolute(config)?;
    ensure!(
        !within_lexical_root(&lexical_config, &lexical_root)
            && !within_lexical_root(&lexical_config, &lexical_absolute(&root)?)
            && !resolved_config_location(config)?.starts_with(&root),
        "configuration must be outside workspace"
    );
    Ok(())
}
fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in std::path::absolute(path)?.components() {
        match component {
            #[cfg(windows)]
            Component::Prefix(prefix) => {
                // Canonical Windows paths use verbatim prefixes. Compare them
                // in the same lexical form as ordinary CLI paths.
                use std::path::Prefix;
                match prefix.kind() {
                    Prefix::VerbatimDisk(drive) | Prefix::Disk(drive) => {
                        normalized.push(format!("{}:", char::from(drive).to_ascii_uppercase()));
                    }
                    Prefix::VerbatimUNC(server, share) | Prefix::UNC(server, share) => {
                        let mut unc = std::ffi::OsString::from(r"\\");
                        unc.push(server);
                        unc.push(r"\");
                        unc.push(share);
                        normalized.push(unc);
                    }
                    _ => normalized.push(component.as_os_str()),
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}
fn within_lexical_root(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut path = path.components();
        root.components().all(|root_component| {
            path.next().is_some_and(|component| {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy())
            })
        })
    }
    #[cfg(not(windows))]
    path.starts_with(root)
}
fn resolved_config_location(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                // Canonicalize the nearest existing ancestor so a missing config
                // beneath a workspace-pointing directory link is rejected too.
                let mut resolved = ancestor.canonicalize()?;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .context("configuration path has no existing ancestor")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .context("configuration path has no existing ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
pub fn capabilities(args: &Args) -> Vec<Capability> {
    let mut c = vec![Capability::ReadWorkspace, Capability::GitRead];
    if args.allow_write {
        c.push(Capability::WriteWorkspace);
    }
    if args.allow_process {
        c.extend([Capability::SpawnProcess, Capability::KillProcess]);
    }
    if args.allow_process_outside_workspace {
        c.push(Capability::OutsideWorkspaceAccess);
    }
    if args.allow_process_network {
        c.push(Capability::NetworkAccess);
    }
    c
}
fn config(args: &Args) -> Result<RuntimeConfig> {
    let p = args.config.as_ref().unwrap();
    Ok(if p.exists() {
        serde_json::from_slice(&std::fs::read(p)?)?
    } else {
        RuntimeConfig::default()
    })
}
fn endpoint(args: &Args) -> String {
    args.endpoint.clone().unwrap_or_else(|| {
        crate::default_endpoint(&args.state_dir.as_ref().unwrap().canonicalize().unwrap())
    })
}
async fn request(args: &Args, payload: Request) -> Result<serde_json::Value> {
    let secret = read_secret(&args.state_dir.as_ref().unwrap().join("ipc.secret"))?;
    Client::new(endpoint(args), secret)
        .request(
            &RequestEnvelope {
                meta: Metadata::new(None, 1500),
                payload,
            },
            |_| {},
        )
        .await?
        .payload
        .map_err(|e| anyhow::anyhow!(e.message))
}
fn verify(args: &Args, h: &serde_json::Value) -> Result<()> {
    let root = args.workspace.as_ref().unwrap().canonicalize()?;
    ensure!(
        h["root"].as_str() == Some(root.to_string_lossy().as_ref()),
        "running daemon root differs"
    );
    let expected: Vec<String> = capabilities(args)
        .iter()
        .map(|c| format!("{c:?}"))
        .collect();
    let mut actual: Vec<String> = serde_json::from_value(h["capabilities"].clone())?;
    actual.sort();
    let mut expected = expected;
    expected.sort();
    ensure!(
        actual == expected,
        "running daemon policy differs; stop it explicitly before connecting"
    );
    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&config(args)?)?));
    ensure!(
        h["config_fingerprint"].as_str() == Some(&hash),
        "running daemon config differs; stop it explicitly before connecting"
    );
    Ok(())
}
pub async fn dispatch(args: &Args) -> Result<bool> {
    let Some(action) = &args.command else {
        return Ok(false);
    };
    match action {
        Action::Serve => return Ok(false),
        Action::Init => {
            let path = args.config.as_ref().unwrap();
            ensure!(!path.exists(), "configuration already exists");
            crate::install::atomic_new(
                path,
                &serde_json::to_vec_pretty(&RuntimeConfig::default())?,
            )?;
            println!("{}", path.display());
        }
        Action::Install { source } | Action::Update { source } => {
            let dir = crate::install::install(source, args.profile.as_ref().unwrap())?;
            println!("{}", dir.display());
        }
        Action::Uninstall => {
            let _lock = offline_lock(args)?;
            let profile = args.profile.as_ref().unwrap();
            #[cfg(windows)]
            if crate::install::defer_self_uninstall(profile)? {
                return Ok(true);
            }
            crate::install::uninstall(profile)?;
        }
        Action::RegisterHost { host, config_file } => {
            crate::host::register(host, config_file, args)?
        }
        Action::Policy {
            action:
                Policy::Approve {
                    program,
                    args: argv,
                    trust_unconfined,
                },
        } => {
            ensure!(*trust_unconfined, "explicit unconfined trust required");
            let program = program.canonicalize()?;
            let fingerprint = ProgramRule::fingerprint(&program)?;
            let mut c = config(args)?;
            c.allowed_programs
                .retain(|r| r.program != program || r.args != *argv);
            c.allowed_programs.push(ProgramRule {
                program,
                args: argv.clone(),
                fingerprint: Some(fingerprint),
                trusted_unconfined: true,
            });
            crate::host::replace_with_backup(
                args.config.as_ref().unwrap(),
                &serde_json::to_vec_pretty(&c)?,
            )?;
            eprintln!(
                "Approved exact program and argv with SHA-256 pin; executable has unconfined access. Restart daemon to apply."
            );
        }
        Action::Policy {
            action: Policy::List,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&config(args)?.allowed_programs)?
            );
        }
        Action::Policy {
            action:
                Policy::Revoke {
                    program,
                    args: argv,
                },
        } => {
            let program = program.canonicalize().unwrap_or_else(|_| program.clone());
            let mut c = config(args)?;
            c.allowed_programs
                .retain(|r| r.program != program || r.args != *argv);
            crate::host::replace_with_backup(
                args.config.as_ref().unwrap(),
                &serde_json::to_vec_pretty(&c)?,
            )?;
            eprintln!("Exact program and argv rule revoked. Restart daemon to apply.");
        }
        Action::Stop => {
            request(args, Request::Shutdown).await?;
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                if let Ok(lock) = offline_lock(args) {
                    drop(lock);
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "daemon is still shutting down; inspect daemon.log"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Action::Doctor => {
            let h = request(args, Request::Heartbeat).await?;
            verify(args, &h)?;
            println!("{}", serde_json::to_string_pretty(&h)?);
        }
        Action::Connect => {
            match request(args, Request::Heartbeat).await {
                Ok(h) => verify(args, &h)?,
                Err(_) => {
                    let mut cmd = Command::new(
                        crate::install::active_executable(args.profile.as_ref().unwrap())?
                            .unwrap_or(std::env::current_exe()?),
                    );
                    cmd.arg("serve");
                    add_args(&mut cmd, args);
                    cmd.stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(std::fs::File::create(
                            args.state_dir.as_ref().unwrap().join("bootstrap.log"),
                        )?);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(0x08000000 | 0x00000200);
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::CommandExt;
                        cmd.process_group(0);
                    }
                    let mut child = cmd.spawn()?;
                    let mut ready = false;
                    for _ in 0..60 {
                        if let Ok(h) = request(args, Request::Heartbeat).await {
                            verify(args, &h)?;
                            ready = true;
                            break;
                        }
                        if child.try_wait()?.is_some() {
                            let log = std::fs::read_to_string(
                                args.state_dir.as_ref().unwrap().join("bootstrap.log"),
                            )
                            .unwrap_or_default();
                            let tail: String = log
                                .chars()
                                .rev()
                                .take(2048)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect();
                            anyhow::bail!(
                                "daemon exited; inspect daemon.log/bootstrap.log: {tail}"
                            );
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    ensure!(ready, "daemon bootstrap timeout; inspect daemon.log");
                }
            }
            let gateway = crate::install::active_executable(args.profile.as_ref().unwrap())?
                .unwrap_or(std::env::current_exe()?)
                .with_file_name(if cfg!(windows) {
                    "mcp-gateway.exe"
                } else {
                    "mcp-gateway"
                });
            let status = Command::new(gateway)
                .arg("--endpoint")
                .arg(endpoint(args))
                .arg("--token-file")
                .arg(args.state_dir.as_ref().unwrap().join("ipc.secret"))
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()?;
            ensure!(status.success(), "gateway exited unsuccessfully");
        }
        Action::Backup { destination } => {
            let _lock = offline_lock(args)?;
            let store = open_store(args).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&Maintenance::new(store).backup(destination).await?)?
            );
        }
        Action::Maintain {
            older_than_ms,
            max_records,
            apply,
            backup,
        } => {
            let _lock = offline_lock(args)?;
            let store = open_store(args).await?;
            let policy = RetentionPolicy {
                older_than_ms: older_than_ms
                    .unwrap_or_else(|| RetentionPolicy::default().older_than_ms),
                max_records: *max_records,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &Maintenance::new(store)
                        .run(&policy, !apply, backup.as_deref())
                        .await?
                )?
            );
        }
    }
    Ok(true)
}
pub fn add_args(cmd: &mut Command, args: &Args) {
    cmd.arg("--profile")
        .arg(args.profile.as_ref().unwrap())
        .arg("--workspace")
        .arg(args.workspace.as_ref().unwrap())
        .arg("--state-dir")
        .arg(args.state_dir.as_ref().unwrap())
        .arg("--config")
        .arg(args.config.as_ref().unwrap());
    if let Some(e) = &args.endpoint {
        cmd.arg("--endpoint").arg(e);
    }
    for (enabled, flag) in [
        (args.allow_write, "--allow-write"),
        (args.allow_process, "--allow-process"),
        (
            args.allow_process_outside_workspace,
            "--allow-process-outside-workspace",
        ),
        (args.allow_process_network, "--allow-process-network"),
    ] {
        if enabled {
            cmd.arg(flag);
        }
    }
}
fn offline_lock(args: &Args) -> Result<std::fs::File> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(args.state_dir.as_ref().unwrap().join("daemon.lock"))?;
    fs2::FileExt::try_lock_exclusive(&f).context("stop daemon before offline maintenance")?;
    Ok(f)
}
async fn open_store(args: &Args) -> Result<LocalStore> {
    let p = args.state_dir.as_ref().unwrap().join("runtime.db");
    ensure!(p.exists(), "runtime database does not exist");
    Ok(LocalStore::open(&p).await?)
}
