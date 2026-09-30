use anyhow::{Result, ensure};
use std::path::Path;
pub fn replace_with_backup(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    ensure!(parent.is_dir(), "configuration parent must exist");
    let old = if path.exists() {
        ensure!(
            !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "configuration cannot be link"
        );
        let old = std::fs::read(path)?;
        let backup = path.with_extension(format!("backup-{}", uuid::Uuid::new_v4()));
        crate::install::atomic_new(&backup, &old)?;
        Some(old)
    } else {
        None
    };
    let temp = parent.join(format!(".runtime-config-{}", uuid::Uuid::new_v4()));
    crate::install::atomic_new(&temp, bytes)?;
    if let Some(old) = old {
        ensure!(
            std::fs::read(path)? == old,
            "configuration changed concurrently"
        );
    }
    #[cfg(windows)]
    {
        // ReplaceFileW is atomic for existing destinations; MoveFileExW for first creation.
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReplaceFileW(
                a: *const u16,
                b: *const u16,
                c: *const u16,
                d: u32,
                e: *mut std::ffi::c_void,
                f: *mut std::ffi::c_void,
            ) -> i32;
            fn MoveFileExW(a: *const u16, b: *const u16, c: u32) -> i32;
        }
        let a: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let ok = unsafe {
            if path.exists() {
                ReplaceFileW(
                    a.as_ptr(),
                    b.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } else {
                MoveFileExW(b.as_ptr(), a.as_ptr(), 8)
            }
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(&temp, path)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn register(host: &str, path: &Path, args: &crate::Args) -> Result<()> {
    let executable = std::env::current_exe()?;
    let mut command = std::process::Command::new(&executable);
    command.arg("connect");
    crate::lifecycle::add_args(&mut command, args);
    let argv: Vec<String> = command
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let bytes = match host {
        "codex" => {
            let text = if path.exists() {
                std::fs::read_to_string(path)?
            } else {
                String::new()
            };
            let mut doc = text.parse::<toml_edit::DocumentMut>()?;
            ensure!(
                doc.get("mcp_servers")
                    .and_then(|v| v.get("local-runtime"))
                    .is_none(),
                "local-runtime host entry already exists; preserve it for explicit review"
            );
            doc["mcp_servers"]["local-runtime"]["command"] =
                toml_edit::value(executable.to_string_lossy().as_ref());
            let mut arr = toml_edit::Array::new();
            for arg in &argv {
                arr.push(arg.as_str());
            }
            doc["mcp_servers"]["local-runtime"]["args"] = toml_edit::value(arr);
            doc.to_string().into_bytes()
        }
        "claude" | "generic" => {
            let mut value = if path.exists() {
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(path)?)?
            } else {
                serde_json::json!({})
            };
            ensure!(value.is_object(), "host config must be JSON object");
            if value.get("mcpServers").is_none() {
                value["mcpServers"] = serde_json::json!({});
            }
            ensure!(
                value["mcpServers"].is_object(),
                "mcpServers must be an object"
            );
            ensure!(
                value["mcpServers"].get("local-runtime").is_none(),
                "local-runtime entry already exists"
            );
            value["mcpServers"]["local-runtime"] =
                serde_json::json!({"command":executable,"args":argv});
            serde_json::to_vec_pretty(&value)?
        }
        _ => anyhow::bail!("host must be codex, claude or generic"),
    };
    replace_with_backup(path, &bytes)?;
    eprintln!(
        "Registered local-runtime at {}; existing configuration backed up. Re-register explicitly after changing installed release.",
        path.display()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_and_backups_configuration() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.json");
        std::fs::write(&p, b"old").unwrap();
        replace_with_backup(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        let backups: Vec<_> = std::fs::read_dir(t.path())
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                p.extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with("backup-")
                    .then_some(p)
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"old");
    }
}
