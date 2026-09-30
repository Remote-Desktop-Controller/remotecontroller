use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub files: BTreeMap<String, String>,
}
pub fn digest(path: &Path) -> Result<String> {
    let mut input = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = input.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn atomic_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn binary_names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["local-daemon.exe", "mcp-gateway.exe"]
    } else {
        ["local-daemon", "mcp-gateway"]
    }
}
pub fn active_executable(profile: &Path) -> Result<Option<PathBuf>> {
    let active = profile.join("active.json");
    if !active.exists() {
        return Ok(None);
    }
    ensure!(
        !std::fs::symlink_metadata(&active)?.file_type().is_symlink(),
        "activation metadata cannot be link"
    );
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(active)?)?;
    let name = v["release"].as_str().context("invalid active release")?;
    ensure!(
        Path::new(name).components().count() == 1 && !name.starts_with('.'),
        "invalid active release path"
    );
    let releases = profile.join("releases").canonicalize()?;
    let release = releases.join(name);
    ensure!(
        !std::fs::symlink_metadata(&release)?
            .file_type()
            .is_symlink()
            && release.canonicalize()?.parent() == Some(releases.as_path()),
        "active release escapes profile"
    );
    let bytes = std::fs::read(release.join("manifest.json"))?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == v["manifest_sha256"].as_str().unwrap_or(""),
        "active manifest changed"
    );
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;
    for (name, hash) in &manifest.files {
        let file = release.join(name);
        ensure!(
            !std::fs::symlink_metadata(&file)?.file_type().is_symlink()
                && digest(&file)?.eq_ignore_ascii_case(hash),
            "installed checksum mismatch"
        );
    }
    Ok(Some(release.join(binary_names()[0])))
}
fn validate_manifest(m: &Manifest) -> Result<()> {
    ensure!(
        !m.version.is_empty()
            && m.version.len() < 80
            && m.version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)),
        "unsafe version"
    );
    ensure!(
        m.files.len() == 2 && binary_names().iter().all(|n| m.files.contains_key(*n)),
        "manifest must contain exactly platform daemon and gateway"
    );
    for hash in m.files.values() {
        ensure!(
            hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()),
            "invalid SHA-256"
        );
    }
    Ok(())
}
pub fn install(source: &Path, profile: &Path) -> Result<PathBuf> {
    ensure!(
        !std::fs::symlink_metadata(source)?.file_type().is_symlink(),
        "bundle cannot be symlink"
    );
    let manifest_path = source.join("manifest.json");
    ensure!(
        !std::fs::symlink_metadata(&manifest_path)?
            .file_type()
            .is_symlink(),
        "manifest cannot be symlink"
    );
    let bytes = std::fs::read(&manifest_path)?;
    ensure!(bytes.len() < 16384, "manifest too large");
    let m: Manifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&m)?;
    for (name, hash) in &m.files {
        let path = source.join(name);
        let meta = std::fs::symlink_metadata(&path)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "bundle member must be regular file"
        );
        ensure!(meta.len() <= 256 * 1024 * 1024, "binary too large");
        ensure!(
            digest(&path)?.eq_ignore_ascii_case(hash),
            "bundle checksum mismatch: {name}"
        );
    }
    let releases = profile.join("releases");
    runtime_transport::private_directory(&releases)?;
    let key = format!("{:x}", Sha256::digest(&bytes));
    let target = releases.join(format!("{}-{}", m.version, &key[..16]));
    ensure!(!target.exists(), "release already installed");
    let staging = releases.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    runtime_transport::private_directory(&staging)?;
    // Verify copied content as well: source modification between preflight and copy cannot activate.
    for (name, hash) in &m.files {
        let dest = staging.join(name);
        let mut input = std::fs::File::open(source.join(name))?;
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&dest)?;
        std::io::copy(&mut input, &mut out)?;
        out.sync_all()?;
        ensure!(
            digest(&dest)?.eq_ignore_ascii_case(hash),
            "source changed during installation"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    atomic_new(&staging.join("manifest.json"), &bytes)?;
    std::fs::rename(&staging, &target)?;
    // The immutable release is complete before activation. Existing release remains available for rollback.
    crate::host::replace_with_backup(
        &profile.join("active.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"release":target.file_name().unwrap().to_string_lossy(),"manifest_sha256":key}),
        )?,
    )?;
    Ok(target)
}
pub fn uninstall(profile: &Path) -> Result<()> {
    let active = profile.join("active.json");
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&active).context("no installed active release")?)?;
    let name = v["release"]
        .as_str()
        .context("invalid installation metadata")?;
    ensure!(
        Path::new(name).components().count() == 1 && !name.starts_with('.'),
        "unsafe installation metadata"
    );
    let releases = profile.join("releases").canonicalize()?;
    let release = releases.join(name);
    ensure!(
        !std::fs::symlink_metadata(&release)?
            .file_type()
            .is_symlink(),
        "release is link"
    );
    ensure!(
        release.canonicalize()?.parent() == Some(releases.as_path()),
        "release escapes install root"
    );
    let bytes = std::fs::read(release.join("manifest.json"))?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == v["manifest_sha256"].as_str().unwrap_or(""),
        "installed manifest changed"
    );
    let m: Manifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&m)?;
    for (name, hash) in &m.files {
        let file = release.join(name);
        ensure!(
            !std::fs::symlink_metadata(&file)?.file_type().is_symlink()
                && digest(&file)?.eq_ignore_ascii_case(hash),
            "installed file changed; preserve for manual review"
        );
    }
    for entry in std::fs::read_dir(&release)? {
        let name = entry?.file_name();
        ensure!(
            name == "manifest.json" || m.files.keys().any(|known| name == known.as_str()),
            "release contains unknown files; preserve for manual review"
        );
    }
    let mut members: Vec<_> = m.files.keys().map(|name| release.join(name)).collect();
    members.push(release.join("manifest.json"));
    #[cfg(windows)]
    delete_windows_members(&members)?;
    #[cfg(not(windows))]
    for file in members {
        std::fs::remove_file(file)?;
    }
    std::fs::remove_dir(&release)?;
    std::fs::remove_file(active)?;
    eprintln!(
        "Active release removed; state, backups, other releases and host registrations preserved."
    );
    Ok(())
}
#[cfg(windows)]
fn delete_windows_members(paths: &[PathBuf]) -> Result<()> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    fn disposition(file: &std::fs::File, delete: bool) -> std::io::Result<()> {
        let flags: u32 = if delete { 1 | 4 } else { 0 }; // DELETE | FORCE_IMAGE_SECTION_CHECK
        // SAFETY: FileDispositionInfoEx takes a four-byte flag word and a live owned handle.
        if unsafe {
            SetFileInformationByHandle(file.as_raw_handle(), 21, (&flags as *const u32).cast(), 4)
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    // Obtain delete access to all members before changing any file. Then mark
    // each for deletion while retaining handles; a busy image fails before
    // close and earlier marks are cancelled, preserving a complete release.
    let handles = paths
        .iter()
        .map(|path| {
            std::fs::OpenOptions::new()
                .access_mode(0x10000)
                .share_mode(7)
                .open(path)
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    for (index, file) in handles.iter().enumerate() {
        if let Err(error) = disposition(file, true) {
            for previous in &handles[..index] {
                disposition(previous, false).context("cancel pending uninstall deletion")?;
            }
            return Err(error.into());
        }
    }
    drop(handles);
    Ok(())
}
#[cfg(windows)]
// This detached Windows worker intentionally outlives its launcher; Windows
// reclaims process objects when handles close and does not create Unix zombies.
#[allow(clippy::zombie_processes)]
pub fn defer_self_uninstall(profile: &Path) -> Result<bool> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let Some(active) = active_executable(profile)? else {
        return Ok(false);
    };
    let current = std::env::current_exe()?.canonicalize()?;
    if current != active.canonicalize()? {
        return Ok(false);
    }
    let worker_directory = profile
        .join("releases")
        .join(format!(".uninstall-{}", uuid::Uuid::new_v4()));
    runtime_transport::private_directory(&worker_directory)?;
    let worker = worker_directory.join("local-daemon.exe");
    std::fs::copy(&current, &worker)?;
    ensure!(
        digest(&worker)? == digest(&current)?,
        "uninstall worker copy changed"
    );
    let log = std::fs::File::create(profile.join("uninstall.log"))?;
    crate::host::replace_with_backup(
        &profile.join("uninstall-status.json"),
        &serde_json::to_vec_pretty(&serde_json::json!({"phase":"scheduled","success":null}))?,
    )?;
    let mut child = Command::new(worker)
        .arg("--finish-uninstall")
        .arg(profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log)
        .creation_flags(0x08000000)
        .spawn()?;
    // EOF tells the separate native worker that this invocation is returning.
    drop(child.stdin.take());
    println!(
        "Uninstall scheduled after this executable exits; result: {}",
        profile.join("uninstall-status.json").display()
    );
    Ok(true)
}
#[cfg(windows)]
pub fn finish_uninstall(profile: &Path) -> Result<()> {
    use std::io::Read;
    ensure!(profile.is_absolute(), "worker profile must be absolute");
    let mut byte = [0u8; 1];
    while std::io::stdin().read(&mut byte)? > 0 {}
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let result = loop {
        match uninstall(profile) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| matches!(io.raw_os_error(), Some(32 | 5)))
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(100))
            }
            result => break result,
        }
    };
    crate::host::replace_with_backup(
        &profile.join("uninstall-status.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"phase":"complete","success":result.is_ok(),"error":result.as_ref().err().map(ToString::to_string)}),
        )?,
    )?;
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bundle(root: &Path) -> PathBuf {
        let p = root.join("source");
        std::fs::create_dir(&p).unwrap();
        let mut files = BTreeMap::new();
        for n in binary_names() {
            std::fs::write(p.join(n), b"candidate").unwrap();
            files.insert(n.to_string(), digest(&p.join(n)).unwrap());
        }
        std::fs::write(
            p.join("manifest.json"),
            serde_json::to_vec(&Manifest {
                version: "0.2.0".into(),
                files,
            })
            .unwrap(),
        )
        .unwrap();
        p
    }
    #[test]
    fn failed_update_preserves_verified_active_release() {
        let t = tempfile::tempdir().unwrap();
        let src = bundle(t.path());
        let profile = t.path().join("profile");
        runtime_transport::private_directory(&profile).unwrap();
        let first = install(&src, &profile).unwrap();
        let active = std::fs::read(profile.join("active.json")).unwrap();
        std::fs::write(src.join(binary_names()[0]), b"tampered").unwrap();
        assert!(install(&src, &profile).is_err());
        assert_eq!(std::fs::read(profile.join("active.json")).unwrap(), active);
        assert_eq!(
            active_executable(&profile).unwrap(),
            Some(first.join(binary_names()[0]).canonicalize().unwrap())
        );
    }
    #[test]
    fn install_verified_uninstall_preserves_state() {
        let t = tempfile::tempdir().unwrap();
        let src = bundle(t.path());
        let profile = t.path().join("profile");
        runtime_transport::private_directory(&profile).unwrap();
        std::fs::write(profile.join("state.keep"), b"data").unwrap();
        let release = install(&src, &profile).unwrap();
        assert!(release.join(binary_names()[0]).exists());
        uninstall(&profile).unwrap();
        assert!(profile.join("state.keep").exists());
    }
    #[test]
    fn rejects_hash_before_copy() {
        let t = tempfile::tempdir().unwrap();
        let src = bundle(t.path());
        std::fs::write(src.join(binary_names()[0]), b"changed").unwrap();
        let profile = t.path().join("profile");
        runtime_transport::private_directory(&profile).unwrap();
        assert!(install(&src, &profile).is_err());
        assert!(!profile.join("active.json").exists());
    }
}
