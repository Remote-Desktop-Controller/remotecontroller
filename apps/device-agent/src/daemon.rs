use crate::profile::Profile;
use anyhow::{Context, Result, ensure};
use runtime_protocol::{Metadata, Request, RequestEnvelope};
use runtime_transport::{Client, read_secret};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

pub struct Daemon {
    pub client: Client,
    child: Option<Child>,
}

pub fn executable(explicit: Option<&Path>) -> Result<PathBuf> {
    let candidate = if let Some(path) = explicit {
        path.to_owned()
    } else {
        std::env::current_exe()?
            .parent()
            .context("Executável sem diretório")?
            .join(if cfg!(windows) {
                "local-daemon.exe"
            } else {
                "local-daemon"
            })
    };
    ensure!(
        candidate.is_file(),
        "local-daemon não encontrado ao lado do agente; use --daemon"
    );
    Ok(candidate.canonicalize()?)
}

pub fn add_args(command: &mut Command, profile: &Profile) {
    command
        .arg("--workspace")
        .arg(&profile.workspace)
        .arg("--state-dir")
        .arg(&profile.state_dir)
        .arg("--endpoint")
        .arg(&profile.endpoint);
    let policy = profile.state_dir.join("runtime.json");
    if policy.exists() {
        command.arg("--config").arg(policy);
    }
    if profile.allow_write {
        command.arg("--allow-write");
    }
    if profile.allow_process {
        command.arg("--allow-process");
    }
    if profile.allow_terminal {
        command.arg("--allow-terminal");
    }
    if profile.allow_process_network {
        command.arg("--allow-process-network");
    }
    if profile.allow_process_outside_workspace {
        command.arg("--allow-process-outside-workspace");
    }
}

async fn connect(profile: &Profile) -> Result<Client> {
    let client = Client::new(
        profile.endpoint.clone(),
        read_secret(&profile.state_dir.join("ipc.secret"))?,
    );
    let reply = client
        .request(
            &RequestEnvelope {
                meta: Metadata::new(None, 1000),
                payload: Request::Heartbeat,
            },
            |_| {},
        )
        .await?;
    let heartbeat = reply.payload.map_err(|e| anyhow::anyhow!(e.message))?;
    ensure!(
        Path::new(
            heartbeat["root"]
                .as_str()
                .context("Heartbeat sem workspace")?
        ) == profile.workspace,
        "Daemon usa outro workspace"
    );
    let capabilities = heartbeat["capabilities"]
        .as_array()
        .context("Heartbeat sem permissões")?;
    for (name, enabled) in [
        ("WriteWorkspace", profile.allow_write),
        ("SpawnProcess", profile.allow_process),
        ("NetworkAccess", profile.allow_process_network),
        (
            "OutsideWorkspaceAccess",
            profile.allow_process_outside_workspace,
        ),
        ("AllowInteractiveTerminal", profile.allow_terminal),
    ] {
        ensure!(
            capabilities.iter().any(|v| v.as_str() == Some(name)) == enabled,
            "Permissões do daemon diferem do perfil; pare o daemon antes de mudar a política"
        );
    }
    Ok(client)
}

impl Daemon {
    /// Restart only the child this agent owns. A borrowed daemon is managed by
    /// its existing owner; never kill or replace it behind that owner's back.
    pub async fn recover_owned(
        &mut self,
        profile: &Profile,
        path: &Path,
        secret: &[u8],
    ) -> Result<bool> {
        let exited = if let Some(child) = self.child.as_mut() {
            child.try_wait()?.is_some()
        } else {
            false
        };
        if !exited {
            return Ok(false);
        }
        ensure!(
            read_secret(&profile.state_dir.join("ipc.secret"))? == secret,
            "Segredo IPC mudou; recuperação automática recusada"
        );
        self.child.take();
        *self = Self::ensure(profile, path).await?;
        Ok(true)
    }

    pub async fn ensure(profile: &Profile, path: &Path) -> Result<Self> {
        runtime_transport::private_directory(&profile.state_dir)?;
        if let Ok(client) = connect(profile).await {
            return Ok(Self {
                client,
                child: None,
            });
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(profile.state_dir.join("agent-bootstrap.log"))?;
        let mut command = Command::new(path);
        command.arg("serve");
        add_args(&mut command, profile);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn()?;
        for _ in 0..100 {
            if let Ok(client) = connect(profile).await {
                return Ok(Self {
                    client,
                    child: Some(child),
                });
            }
            ensure!(
                child.try_wait()?.is_none(),
                "Daemon encerrou; veja agent-bootstrap.log e daemon.log"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        child.kill().await?;
        anyhow::bail!("Daemon não iniciou; consulte os logs privados")
    }

    pub async fn stop(mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = self
                .client
                .request(
                    &RequestEnvelope {
                        meta: Metadata::new(None, 3000),
                        payload: Request::Shutdown,
                    },
                    |_| {},
                )
                .await;
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                child.kill().await?;
                child.wait().await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_daemon_crash_recovers_without_rotating_secret_or_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let state = fixture.path().join("private-state");
        #[cfg(windows)]
        let endpoint = format!(r"\\.\pipe\rdc-recovery-{}", uuid::Uuid::new_v4());
        #[cfg(unix)]
        let endpoint = state.join("daemon.sock").to_string_lossy().into_owned();
        let config = Profile {
            server: "https://example.test".into(),
            device_id: uuid::Uuid::new_v4(),
            device_token: "x".repeat(32),
            workspace: workspace.canonicalize().unwrap(),
            state_dir: state,
            endpoint,
            allow_http_loopback: false,
            allow_write: false,
            allow_process: false,
            allow_terminal: false,
            allow_process_network: false,
            allow_process_outside_workspace: false,
        };
        let binaries = std::env::var_os("RUNTIME_TEST_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug")
            });
        let executable = binaries.join(if cfg!(windows) {
            "local-daemon.exe"
        } else {
            "local-daemon"
        });
        let mut daemon = Daemon::ensure(&config, &executable).await.unwrap();
        let secret = read_secret(&config.state_dir.join("ipc.secret")).unwrap();
        let (workspace_id, _) = crate::runner::catalog(&daemon.client).await.unwrap();
        daemon.child.as_mut().unwrap().kill().await.unwrap();
        daemon.child.as_mut().unwrap().wait().await.unwrap();
        assert!(
            daemon
                .recover_owned(&config, &executable, &secret)
                .await
                .unwrap()
        );
        assert_eq!(
            read_secret(&config.state_dir.join("ipc.secret")).unwrap(),
            secret
        );
        assert_eq!(
            crate::runner::catalog(&daemon.client).await.unwrap().0,
            workspace_id
        );
        assert!(
            !daemon
                .recover_owned(&config, &executable, &secret)
                .await
                .unwrap()
        );
        daemon.stop().await.unwrap();
    }
}
