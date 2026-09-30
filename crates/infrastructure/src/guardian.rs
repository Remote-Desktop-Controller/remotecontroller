//! Unix process supervisor. Executed in a separate native helper process.
#[cfg(unix)]
pub async fn run(arguments: Vec<std::ffi::OsString>) -> std::io::Result<i32> {
    let (program, args) = arguments.split_first().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "guardian requires absolute program",
        )
    })?;
    if !std::path::Path::new(program).is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "guardian requires absolute program",
        ));
    }
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let pid = child
        .id()
        .ok_or_else(|| std::io::Error::other("guardian child lacks PID"))?;
    let (sender, disconnected) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut input = std::io::stdin().lock();
        let mut byte = [0u8; 1];
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = sender.send(());
    });
    let result = tokio::select! {
        status=child.wait()=>status,
        _=disconnected=>{
            // SAFETY: group was created by process_group(0), not recovered from persisted PID.
            unsafe{libc::kill(-(pid as i32),libc::SIGKILL);}
            let _=child.kill().await;
            child.wait().await
        }
    };
    // A target may exit while descendants retain pipe handles; retire its live group.
    // SAFETY: this is the group owned for this single target lifecycle.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    Ok(result?.code().unwrap_or(128))
}
