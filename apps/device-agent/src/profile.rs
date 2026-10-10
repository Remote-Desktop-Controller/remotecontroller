use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub server: String,
    pub device_id: uuid::Uuid,
    pub device_token: String,
    pub workspace: PathBuf,
    pub state_dir: PathBuf,
    pub endpoint: String,
    #[serde(default)]
    pub allow_http_loopback: bool,
    #[serde(default)]
    pub allow_write: bool,
    #[serde(default)]
    pub allow_process: bool,
    #[serde(default)]
    pub allow_terminal: bool,
    #[serde(default)]
    pub allow_process_network: bool,
    #[serde(default)]
    pub allow_process_outside_workspace: bool,
    #[serde(default)]
    pub full_computer: bool,
}

pub fn protect_profile(profile: &Path, workspace: &Path) -> Result<PathBuf> {
    protect_profile_scoped(profile, workspace, false)
}

pub fn protect_profile_scoped(
    profile: &Path,
    workspace: &Path,
    full_computer: bool,
) -> Result<PathBuf> {
    let workspace = workspace.canonicalize()?;
    let absolute = std::path::absolute(profile)?;
    ensure!(
        !absolute
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "Perfil não pode conter segmentos .."
    );
    let mut ancestor = absolute.clone();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .context("Perfil sem ancestral existente")?
                .to_owned(),
        );
        ensure!(ancestor.pop(), "Perfil sem ancestral existente");
    }
    let mut resolved = ancestor.canonicalize()?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    if !full_computer {
        ensure!(
            !resolved.starts_with(&workspace),
            "Perfil privado precisa ficar fora do workspace"
        );
    }
    runtime_transport::private_directory(profile)?;
    let profile = profile.canonicalize()?;
    if !full_computer {
        ensure!(
            !profile.starts_with(&workspace),
            "Perfil privado precisa ficar fora do workspace"
        );
    }
    Ok(profile)
}

pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Caminho privado sem diretório")?;
    runtime_transport::private_directory(parent)?;
    ensure!(
        !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
        "Arquivo privado não pode ser link"
    );
    let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // Destination replacement is refused: credentials are not silently rotated.
    let result = fs::hard_link(&temp, path);
    fs::remove_file(&temp)?;
    result?;
    #[cfg(unix)]
    {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub fn save(path: &Path, profile: &Profile) -> Result<()> {
    crate::validate_server(&profile.server, profile.allow_http_loopback)?;
    write_private(path, &serde_json::to_vec_pretty(profile)?)
}

pub fn load(path: &Path) -> Result<Profile> {
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Perfil não pode ser link"
    );
    let bytes = fs::read(path)?;
    ensure!(bytes.len() <= 16 * 1024, "Perfil excede limite");
    let profile: Profile = serde_json::from_slice(&bytes)?;
    crate::validate_server(&profile.server, profile.allow_http_loopback)?;
    ensure!(
        profile.device_token.len() >= 32,
        "Credencial de dispositivo inválida"
    );
    if !profile.full_computer {
        let parent = path.parent().context("Perfil sem parent")?.canonicalize()?;
        ensure!(
            !parent.starts_with(profile.workspace.canonicalize()?),
            "Perfil dentro do workspace"
        );
    }
    Ok(profile)
}

pub fn update(path: &Path, profile: &Profile) -> Result<()> {
    let previous = load(path)?;
    ensure!(
        previous.device_id == profile.device_id
            && previous.device_token == profile.device_token
            && previous.workspace == profile.workspace
            && previous.server == profile.server,
        "Atualização de permissões não pode substituir identidade"
    );
    let parent = path.parent().context("Perfil sem parent")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join("agent.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Pare o agente antes de alterar permissões")?;
    let temporary = parent.join(format!(".profile-{}.json", uuid::Uuid::new_v4()));
    write_private(&temporary, &serde_json::to_vec_pretty(profile)?)?;
    fs::rename(temporary, path)?;
    #[cfg(unix)]
    {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
