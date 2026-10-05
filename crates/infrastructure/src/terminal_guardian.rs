//! This helper contains itself before the trusted shell is created.
use std::{
    ffi::OsString,
    path::Path,
    process::{Command, Stdio},
};
#[cfg(unix)]
pub(crate) fn kill_tree(root: i32) {
    kill_session_children(root);
    unsafe {
        libc::kill(-root, libc::SIGKILL);
    }
}
#[cfg(unix)]
fn kill_session_children(root: i32) {
    // Job control splits process groups and shell exit can reparent background
    // children. The live, owned PTY session remains the reliable boundary.
    if unsafe { libc::getsid(root) } != root {
        return;
    }
    if let Ok(list) = Command::new("/bin/ps").args(["-axo", "pid="]).output() {
        let pids: Vec<i32> = String::from_utf8_lossy(&list.stdout)
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        for pid in pids {
            if pid != root && unsafe { libc::getsid(pid) } == root {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}
pub fn run(arguments: Vec<OsString>) -> std::io::Result<i32> {
    if arguments.len() < 3 {
        return Err(std::io::Error::other("terminal guardian arguments missing"));
    }
    let parent: u32 = arguments[0]
        .to_string_lossy()
        .parse()
        .map_err(|_| std::io::Error::other("invalid parent PID"))?;
    let program = Path::new(&arguments[1]);
    if !program.is_absolute() {
        return Err(std::io::Error::other("absolute terminal program required"));
    }
    let actual = crate::processes::ProgramRule::fingerprint(program)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    if actual != arguments[2].to_string_lossy() {
        return Err(std::io::Error::other(
            "terminal executable fingerprint changed",
        ));
    }
    #[cfg(windows)]
    let _job =
        std::mem::ManuallyDrop::new(crate::processes::platform::Job::attach(std::process::id())?);
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let handle = handle as usize;
        // The Job remains open in this process until exit; all child shells inherit it.
        std::thread::spawn(move || {
            unsafe {
                WaitForSingleObject(handle as _, u32::MAX);
                CloseHandle(handle as _);
            }
            std::process::exit(128);
        });
    }
    #[cfg(unix)]
    {
        let group = unsafe { libc::getpgrp() };
        if group != std::process::id() as i32 {
            return Err(std::io::Error::other(
                "PTY guardian must own its process group",
            ));
        }
        if unsafe { libc::getppid() } != parent as i32 {
            return Err(std::io::Error::other(
                "terminal parent already disconnected",
            ));
        }
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if unsafe { libc::getppid() } != parent as i32 {
                    kill_tree(group);
                    std::process::exit(128);
                }
            }
        });
    }
    let mut command = Command::new(program);
    command.args(&arguments[3..]);
    #[cfg(windows)]
    {
        // ConPTY attaches a console but the helper's inherited std handles may
        // be NUL. Reopen that console's devices before spawning the trusted CUI.
        let input = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONIN$")?;
        let output = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("CONOUT$")?;
        command
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(output.try_clone()?))
            .stderr(Stdio::from(output));
    }
    #[cfg(unix)]
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command.spawn()?;
    let status = child.wait()?;
    // On Windows OS process exit closes the Job, including error paths. Dropping
    // it in Rust would kill the helper before its error/exit status is reported.
    #[cfg(unix)]
    {
        // Retire every group in the owned session, including reparented jobs.
        kill_session_children(std::process::id() as i32);
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::kill(-libc::getpgrp(), libc::SIGTERM);
        }
    }
    Ok(status.code().unwrap_or(128))
}
