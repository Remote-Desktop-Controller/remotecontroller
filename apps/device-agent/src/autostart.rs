use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

pub fn configure(profile: &Path, enable: bool) -> Result<()> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let profile = if profile.exists() {
        profile.canonicalize()?
    } else {
        std::path::absolute(profile)?
    };
    let identity = format!(
        "remotecontroller-{:x}",
        Sha256::digest(profile.to_string_lossy().as_bytes())
    );
    let identity = &identity[..33];
    #[cfg(windows)]
    {
        use base64::Engine;
        let mut command = Command::new("reg.exe");
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        if enable {
            let literal = |value: &str| format!("'{}'", value.replace('\'', "''"));
            let script = format!(
                "Start-Process -FilePath {} -ArgumentList '--profile', {}, 'run' -WindowStyle Hidden",
                literal(&executable.to_string_lossy()),
                literal(&format!("\"{}\"", profile.display()))
            );
            let bytes = script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let powershell = std::path::PathBuf::from(
                std::env::var_os("SystemRoot")
                    .ok_or_else(|| anyhow::anyhow!("SystemRoot indisponível"))?,
            )
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let data = format!(
                "\"{}\" -NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand {encoded}",
                powershell.display()
            );
            command.args([
                "add", key, "/v", identity, "/t", "REG_SZ", "/d", &data, "/f",
            ]);
        } else {
            command.args(["delete", key, "/v", identity, "/f"]);
        }
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        let output = command.output()?;
        if enable {
            ensure!(
                output.status.success(),
                "Não foi possível atualizar inicialização automática"
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME indisponível"))?;
        let directory = std::path::PathBuf::from(home).join(".config/systemd/user");
        std::fs::create_dir_all(&directory)?;
        let name = format!("{identity}.service");
        let quote = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        };
        let path = directory.join(&name);
        if enable {
            std::fs::write(
                &path,
                format!(
                    "[Unit]\nDescription=Remote Desktop Controller\nAfter=network-online.target\n[Service]\nExecStart=\"{}\" --profile \"{}\" run\nRestart=on-failure\nRestartSec=5\n[Install]\nWantedBy=default.target\n",
                    quote(&executable),
                    quote(&profile)
                ),
            )?;
        }
        ensure!(
            Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status()?
                .success(),
            "systemd user indisponível"
        );
        let action = if enable { "enable" } else { "disable" };
        ensure!(
            Command::new("systemctl")
                .args(["--user", action, "--now", &name])
                .status()?
                .success(),
            "Falha em systemd user"
        );
        if !enable && path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME indisponível"))?;
        let directory = std::path::PathBuf::from(home).join("Library/LaunchAgents");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{identity}.plist"));
        let xml = |value: &str| {
            value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?
            .trim()
            .to_owned();
        if enable {
            std::fs::write(
                &path,
                format!(
                    "<?xml version=\"1.0\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{identity}</string><key>ProgramArguments</key><array><string>{}</string><string>--profile</string><string>{}</string><string>run</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/></dict></plist>",
                    xml(&executable.to_string_lossy()),
                    xml(&profile.to_string_lossy())
                ),
            )?;
            ensure!(
                Command::new("launchctl")
                    .args(["bootstrap", &format!("gui/{uid}")])
                    .arg(&path)
                    .status()?
                    .success(),
                "Falha ao iniciar LaunchAgent"
            );
        } else {
            ensure!(
                Command::new("launchctl")
                    .args(["bootout", &format!("gui/{uid}/{identity}")])
                    .status()?
                    .success(),
                "Falha ao remover LaunchAgent"
            );
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}
