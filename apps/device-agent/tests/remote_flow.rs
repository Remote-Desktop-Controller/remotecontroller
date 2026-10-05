use anyhow::{Context, Result, ensure};
use remote_service::{Config, application};
use reqwest::{Client, redirect::Policy};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

fn binary(name: &str) -> PathBuf {
    let dir = std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug"));
    dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}
fn process(name: &str) -> Command {
    let mut cmd = Command::new(binary(name));
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}
struct StopOnDrop(PathBuf);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if !self.0.join("agent.json").exists() {
            return;
        }
        let mut command = std::process::Command::new(binary("device-agent"));
        command
            .arg("--profile")
            .arg(&self.0)
            .arg("stop")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let _ = command.status();
    }
}
async fn checked(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let value: Value = response.json().await?;
    ensure!(status.is_success(), "HTTP {status}: {value}");
    Ok(value)
}
async fn mcp(client: &Client, origin: &str, token: &str, request: Value) -> Result<Value> {
    checked(
        client
            .post(format!("{origin}/mcp"))
            .bearer_auth(token)
            .json(&request)
            .send()
            .await?,
    )
    .await
}

#[tokio::test]
async fn real_agent_pairing_oauth_read_write_session_retry_restart_and_revocation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace)?;
    std::fs::write(workspace.join("hello.txt"), "Olá, computador remoto!")?;
    let profile = dir.path().join("agent");
    let _cleanup = StopOnDrop(profile.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let callback = "https://test.example/callback";
    let mut config = Config::new(origin.clone(), true, vec![callback.into()])?;
    config.poll_seconds = 25;
    config.call_seconds = 5;
    let app = application(&dir.path().join("service.db"), config).await?;
    let service = tokio::spawn(async move { axum::serve(listener, app.into_make_service()).await });
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let registered = client
        .post(format!("{origin}/api/auth/register"))
        .json(
            &json!({"email":"owner@example.test","password":"Long-password-for-integration-527!"}),
        )
        .send()
        .await?;
    let cookie = registered
        .headers()
        .get("set-cookie")
        .context("Missing cookie")?
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let registration = checked(registered).await?;
    let csrf = registration["csrf_token"]
        .as_str()
        .context("Missing CSRF")?;
    let mut pair = process("device-agent")
        .arg("--profile")
        .arg(&profile)
        .arg("--daemon")
        .arg(binary("local-daemon"))
        .arg("pair")
        .arg("--server")
        .arg(&origin)
        .arg("--workspace")
        .arg(&workspace)
        .args(["--allow-http-loopback", "--allow-write"])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = pair.stdout.take().context("Pair stdout missing")?;
    let mut lines = BufReader::new(stdout).lines();
    let pairing_line = tokio::time::timeout(Duration::from_secs(25), async {
        while let Some(line) = lines.next_line().await? {
            if line.starts_with("Abra ") {
                return Ok::<_, anyhow::Error>(line);
            }
        }
        anyhow::bail!("Pairing exited before code")
    })
    .await??;
    let code = pairing_line
        .split_whitespace()
        .last()
        .context("Code missing")?;
    checked(
        client
            .post(format!("{origin}/api/pairings/approve"))
            .header("cookie", &cookie)
            .header("x-csrf-token", csrf)
            .json(&json!({"code":code}))
            .send()
            .await?,
    )
    .await?;
    ensure!(
        tokio::time::timeout(Duration::from_secs(15), pair.wait())
            .await??
            .success(),
        "Pair command failed"
    );
    let credentials = device_agent::profile::load(&profile.join("agent.json"))?;
    ensure!(
        !credentials.device_token.is_empty(),
        "No device token persisted"
    );
    let mut agent = process("device-agent")
        .arg("--profile")
        .arg(&profile)
        .arg("--daemon")
        .arg(binary("local-daemon"))
        .arg("run")
        .spawn()?;
    let registered_client = checked(
        client
            .post(format!("{origin}/oauth/register"))
            .json(&json!({"client_name":"Integration client","redirect_uris":[callback]}))
            .send()
            .await?,
    )
    .await?;
    let client_id = registered_client["client_id"]
        .as_str()
        .context("Missing client id")?;
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = remote_service::security::hash(verifier);
    let resource = format!("{origin}/mcp");
    let authorization = client
        .post(format!("{origin}/oauth/authorize"))
        .header("cookie", &cookie)
        .form(&[
            ("client_id", client_id),
            ("redirect_uri", callback),
            ("response_type", "code"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("resource", &resource),
            ("scope", "devices:read tools:read tools:execute"),
            ("state", "integration"),
            ("csrf", csrf),
            ("decision", "allow"),
        ])
        .send()
        .await?;
    ensure!(
        authorization.status() == reqwest::StatusCode::SEE_OTHER,
        "Authorize status {}",
        authorization.status()
    );
    let redirect = url::Url::parse(
        authorization
            .headers()
            .get("location")
            .context("Missing redirect")?
            .to_str()?,
    )?;
    let code = redirect
        .query_pairs()
        .find(|(key, _)| key == "code")
        .context("Missing auth code")?
        .1
        .into_owned();
    let tokens = checked(
        client
            .post(format!("{origin}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", &code),
                ("code_verifier", verifier),
                ("redirect_uri", callback),
                ("resource", &resource),
            ])
            .send()
            .await?,
    )
    .await?;
    let token = tokens["access_token"].as_str().context("Missing token")?;
    let initialized=mcp(&client,&origin,token,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"integration","version":"1"}}})).await?;
    ensure!(
        initialized["result"]["protocolVersion"].is_string(),
        "Initialize failed: {initialized}"
    );
    let selected=mcp(&client,&origin,token,json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"device.select","arguments":{"device_id":credentials.device_id}}})).await?;
    ensure!(
        selected["error"].is_null(),
        "Device selection failed: {selected}"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let devices=mcp(&client,&origin,token,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"device.list","arguments":{}}})).await?;
        if devices.to_string().contains("\"online\":true") {
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Device never became online: {devices}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let operation = uuid::Uuid::new_v4();
    let read_request = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"filesystem.read","arguments":{"path":"hello.txt"},"_meta":{"runtimeOperationId":operation}}});
    let read = mcp(&client, &origin, token, read_request).await?;
    ensure!(
        read.to_string().contains("Olá, computador remoto!"),
        "Remote read failed: {read}"
    );
    let created=mcp(&client,&origin,token,json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"session.create","arguments":{"label":"Remote task"}}})).await?;
    let session = created["result"]["structuredContent"]["result"]["result"]["session_id"]
        .as_str()
        .context(format!("Missing session: {created}"))?;
    let write_operation = uuid::Uuid::new_v4();
    let write_request = json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"filesystem.write","arguments":{"path":"remote.txt","content":"Persistente","session_id":session},"_meta":{"runtimeOperationId":write_operation}}});
    let written = mcp(&client, &origin, token, write_request.clone()).await?;
    ensure!(
        written["result"]["isError"] != true,
        "Remote write failed: {written}"
    );
    ensure!(
        std::fs::read_to_string(workspace.join("remote.txt"))? == "Persistente",
        "Remote bytes mismatch"
    );
    std::fs::write(workspace.join("remote.txt"), "Mudança externa preservada")?;
    let replay = mcp(&client, &origin, token, write_request.clone()).await?;
    ensure!(
        replay["result"]["isError"] != true,
        "Replay failed: {replay}"
    );
    ensure!(
        std::fs::read_to_string(workspace.join("remote.txt"))? == "Mudança externa preservada",
        "Replay repeated write"
    );
    ensure!(
        process("device-agent")
            .arg("--profile")
            .arg(&profile)
            .arg("stop")
            .status()
            .await?
            .success(),
        "Stop failed"
    );
    ensure!(
        tokio::time::timeout(Duration::from_secs(10), agent.wait())
            .await??
            .success(),
        "Agent stop failed"
    );
    agent = process("device-agent")
        .arg("--profile")
        .arg(&profile)
        .arg("--daemon")
        .arg(binary("local-daemon"))
        .arg("run")
        .spawn()?;
    let restarted = mcp(&client, &origin, token, write_request).await?;
    ensure!(
        restarted["result"]["isError"] != true,
        "Restart replay failed: {restarted}"
    );
    ensure!(
        std::fs::read_to_string(workspace.join("remote.txt"))? == "Mudança externa preservada",
        "Restart replay repeated effect"
    );
    checked(
        client
            .post(format!(
                "{origin}/api/devices/{}/revoke",
                credentials.device_id
            ))
            .header("cookie", &cookie)
            .header("x-csrf-token", csrf)
            .json(&json!({}))
            .send()
            .await?,
    )
    .await?;
    let rejected = client
        .post(format!("{origin}/api/agent/heartbeat"))
        .bearer_auth(&credentials.device_token)
        .json(&json!({"workspace_id":uuid::Uuid::nil(),"tools":[]}))
        .send()
        .await?;
    ensure!(
        rejected.status() == reqwest::StatusCode::UNAUTHORIZED,
        "Revoked device still authorized"
    );
    let _ = process("device-agent")
        .arg("--profile")
        .arg(&profile)
        .arg("stop")
        .status()
        .await;
    let _ = tokio::time::timeout(Duration::from_secs(10), agent.wait()).await;
    service.abort();
    Ok(())
}
