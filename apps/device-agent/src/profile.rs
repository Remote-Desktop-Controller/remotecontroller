use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    if let Some(parent) = resolved.parent().filter(|p| !p.as_os_str().is_empty() && !p.exists()) {
        let _ = runtime_transport::private_directory(parent);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileNotFoundError {
    pub path: PathBuf,
}

impl std::fmt::Display for ProfileNotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dispositivo não pareado. Perfil não encontrado em: {}",
            self.path.display()
        )
    }
}

impl std::error::Error for ProfileNotFoundError {}

pub fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<ProfileNotFoundError>().is_some()
        || err
            .root_cause()
            .downcast_ref::<ProfileNotFoundError>()
            .is_some()
        || err
            .root_cause()
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

pub fn has_profile(root: &Path) -> bool {
    let credential = root.join("agent.json");
    fs::symlink_metadata(&credential)
        .map(|m| !m.file_type().is_symlink() && m.is_file())
        .unwrap_or(false)
}

pub fn default_profile() -> Result<PathBuf> {
    default_profile_from_env(
        std::env::var_os("HOME").as_deref(),
        std::env::var_os("USERPROFILE").as_deref(),
        std::env::var_os("LOCALAPPDATA").as_deref(),
    )
}

pub fn default_profile_from_env(
    home_env: Option<&std::ffi::OsStr>,
    userprofile_env: Option<&std::ffi::OsStr>,
    localappdata_env: Option<&std::ffi::OsStr>,
) -> Result<PathBuf> {
    let home = home_env
        .filter(|s| !s.is_empty())
        .or_else(|| userprofile_env.filter(|s| !s.is_empty()))
        .map(PathBuf::from);

    let canonical_agent = home
        .as_ref()
        .map(|h| h.join(".remotecontroller").join("agent"));

    // 1. Verificar se $HOME\.remotecontroller\agent\agent.json existe
    if let Some(canonical) = canonical_agent.as_ref().filter(|c| has_profile(c)) {
        return Ok(canonical.clone());
    }

    // 2. Se não existir, verificar se %LOCALAPPDATA%\RemoteDesktopController\agent\agent.json existe (fallback retroativo)
    let legacy_agent = if cfg!(windows) {
        localappdata_env
            .filter(|s| !s.is_empty())
            .map(|l| PathBuf::from(l).join("RemoteDesktopController").join("agent"))
    } else {
        home.as_ref()
            .map(|h| h.join(".local").join("share").join("remotecontroller").join("agent"))
    };

    if let Some(legacy) = legacy_agent.as_ref().filter(|l| has_profile(l)) {
        return Ok(legacy.clone());
    }

    // 3. Caso nenhum exista, adotar $HOME\.remotecontroller\agent como padrão para novos perfis, mantendo a permissão privada.
    if let Some(canonical) = canonical_agent {
        Ok(canonical)
    } else if let Some(legacy) = legacy_agent {
        Ok(legacy)
    } else {
        anyhow::bail!("Diretório base do usuário indisponível (HOME/USERPROFILE); use --profile");
    }
}

pub fn load(path: &Path) -> Result<Profile> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProfileNotFoundError {
                path: path.to_path_buf(),
            }
            .into());
        }
        Err(err) => return Err(err).context("Falha ao ler metadados do perfil"),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "Perfil não pode ser link"
    );
    ensure!(
        metadata.is_file(),
        "Perfil precisa ser um arquivo regular: {}",
        path.display()
    );
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProfileNotFoundError {
                path: path.to_path_buf(),
            }
            .into());
        }
        Err(err) => return Err(err).context("Falha ao ler arquivo de perfil"),
    };
    ensure!(bytes.len() <= 16 * 1024, "Perfil excede limite");
    let content = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        &bytes[..]
    };
    let profile: Profile = serde_json::from_slice(content)?;
    crate::validate_server(&profile.server, profile.allow_http_loopback)?;
    ensure!(
        profile.device_token.len() >= 32,
        "Credencial de dispositivo inválida"
    );
    if !profile.full_computer {
        let parent = path
            .parent()
            .map(|p| if p.as_os_str().is_empty() { Path::new(".") } else { p })
            .context("Perfil sem parent")?
            .canonicalize()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_nonexistent_profile_returns_typed_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.json");
        let result = load(&path);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(is_not_found(&err), "is_not_found must be true");
        let typed = err.downcast_ref::<ProfileNotFoundError>();
        assert!(typed.is_some(), "Must downcast to ProfileNotFoundError");
        assert_eq!(typed.unwrap().path, path);
        let msg = err.to_string();
        assert!(
            msg.contains("Dispositivo não pareado"),
            "Display message must be human clear: {msg}"
        );
        assert!(!msg.contains("os error 2"), "Must not be raw OS error: {msg}");
    }

    #[test]
    fn default_profile_prioritizes_canonical_when_present() {
        let home = tempfile::tempdir().unwrap();
        let localappdata = tempfile::tempdir().unwrap();

        let canonical_dir = home.path().join(".remotecontroller").join("agent");
        std::fs::create_dir_all(&canonical_dir).unwrap();
        std::fs::write(canonical_dir.join("agent.json"), b"{}").unwrap();

        let legacy_dir = localappdata
            .path()
            .join("RemoteDesktopController")
            .join("agent");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("agent.json"), b"{}").unwrap();

        let resolved = default_profile_from_env(
            Some(home.path().as_os_str()),
            None,
            Some(localappdata.path().as_os_str()),
        )
        .unwrap();

        assert_eq!(resolved, canonical_dir);
    }

    #[test]
    fn default_profile_falls_back_to_legacy_when_canonical_missing() {
        let home = tempfile::tempdir().unwrap();
        let localappdata = tempfile::tempdir().unwrap();

        let legacy_dir = if cfg!(windows) {
            localappdata
                .path()
                .join("RemoteDesktopController")
                .join("agent")
        } else {
            home.path()
                .join(".local")
                .join("share")
                .join("remotecontroller")
                .join("agent")
        };
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("agent.json"), b"{}").unwrap();

        let resolved = default_profile_from_env(
            Some(home.path().as_os_str()),
            None,
            Some(localappdata.path().as_os_str()),
        )
        .unwrap();

        assert_eq!(resolved, legacy_dir);
    }

    #[test]
    fn default_profile_adopts_canonical_when_neither_exists() {
        let home = tempfile::tempdir().unwrap();
        let localappdata = tempfile::tempdir().unwrap();

        let resolved = default_profile_from_env(
            Some(home.path().as_os_str()),
            None,
            Some(localappdata.path().as_os_str()),
        )
        .unwrap();

        let expected = home.path().join(".remotecontroller").join("agent");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn default_profile_ignores_empty_home_and_uses_userprofile() {
        let userprofile = tempfile::tempdir().unwrap();
        let localappdata = tempfile::tempdir().unwrap();

        let resolved = default_profile_from_env(
            Some(std::ffi::OsStr::new("")),
            Some(userprofile.path().as_os_str()),
            Some(localappdata.path().as_os_str()),
        )
        .unwrap();

        let expected = userprofile.path().join(".remotecontroller").join("agent");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn default_profile_bails_when_all_envs_empty_or_none() {
        let result = default_profile_from_env(
            Some(std::ffi::OsStr::new("")),
            Some(std::ffi::OsStr::new("")),
            Some(std::ffi::OsStr::new("")),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Diretório base do usuário indisponível"));
    }

    #[test]
    fn load_directory_fails_with_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = load(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(!is_not_found(&err), "A directory is not a missing profile");
        let msg = err.to_string();
        assert!(
            msg.contains("arquivo regular"),
            "Error must specify that profile must be a regular file: {msg}"
        );
    }
}
