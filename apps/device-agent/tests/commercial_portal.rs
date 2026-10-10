//! Joined local integration. The portal test harness owns the disposable PGlite/
//! provider fixture and invokes this test with RDC_E2E_PORTAL_ORIGIN. Nothing here
//! reaches production. Execution uses a real local-daemon and a temporary file.
use anyhow::{Context, Result, ensure};
use device_agent::{daemon::Daemon, profile::Profile, runner};
use libsql::params;
use remote_service::{CommercialConfig, Config, application, security};
use reqwest::{Client, redirect::Policy};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const SUBJECT: &str = "11111111-1111-4111-8111-111111111111";
const PUBLIC: &str = "https://remote.fixture.test";
async fn checked(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body: Value = response.json().await?;
    ensure!(status.is_success(), "fixture HTTP {status}: {body}");
    Ok(body)
}

#[tokio::test]
#[ignore = "requires joined portal/PGlite HTTP harness"]
async fn real_rust_and_agent_use_portal_sql_license_on_first_remote_operation() -> Result<()> {
    let portal = std::env::var("RDC_E2E_PORTAL_ORIGIN")
        .context("requires Vitest e2e-commercial HTTP harness")?;
    ensure!(
        portal.starts_with("http://127.0.0.1:"),
        "fixture must be local"
    );
    let dir = tempfile::tempdir()?;
    let dbpath = dir.path().join("service.db");
    let web = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", web.local_addr()?);
    let mut config = Config::new(PUBLIC.into(), false, vec![])?;
    config.commercial = Some(CommercialConfig::new(portal.clone(), "s".repeat(32), true)?);
    config.poll_seconds = 2;
    config.call_seconds = 5;
    let app = application(&dbpath, config).await?;
    let webtask = tokio::spawn(async move { axum::serve(web, app.into_make_service()).await });
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let login = client.get(format!("{origin}/login")).send().await?;
    ensure!(login.status().is_redirection());
    let nonce_cookie = login
        .headers()
        .get("set-cookie")
        .context("nonce cookie")?
        .to_str()?
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let redirect = url::Url::parse(
        login
            .headers()
            .get("location")
            .context("portal redirect")?
            .to_str()?,
    )?;
    let state = redirect
        .query_pairs()
        .find(|(k, _)| k == "state")
        .context("state")?
        .1
        .into_owned();
    let connect = client
        .get(format!("{portal}/api/mcp/connect"))
        .header("x-fixture-user", SUBJECT)
        .query(&[
            ("state", state.as_str()),
            (
                "redirect_uri",
                &format!("{PUBLIC}/auth/commercial/callback"),
            ),
        ])
        .send()
        .await?;
    let connected = checked(connect).await?;
    let callback = url::Url::parse(
        connected["redirectUrl"]
            .as_str()
            .context("callback redirectUrl")?,
    )?;
    let logged = client
        .get(format!(
            "{origin}{}?{}",
            callback.path(),
            callback.query().unwrap_or_default()
        ))
        .header("cookie", nonce_cookie)
        .send()
        .await?;
    ensure!(
        logged.status().is_redirection(),
        "Rust exchange {}",
        logged.status()
    );

    // OAuth grant/device are operational fixtures; identity is never client seeded.
    let database = libsql::Builder::new_local(&dbpath).build().await?;
    let conn = database.connect()?;
    let owner: String = conn
        .query(
            "SELECT user_id FROM commercial_accounts WHERE subject=?1",
            params![SUBJECT],
        )
        .await?
        .next()
        .await?
        .context("trusted subject binding")?
        .get(0)?;
    let endpoint = if cfg!(windows) {
        format!(r"\\.\pipe\commercial-agent-{}", uuid::Uuid::new_v4())
    } else {
        dir.path().join("ipc.sock").to_string_lossy().into_owned()
    };
    let root = dir.path().join("private");
    runtime_transport::private_directory(&root)?;
    let workspace_path = dir.path().join("workspace");
    std::fs::create_dir(&workspace_path)?;
    std::fs::write(workspace_path.join("proof.txt"), "joined commercial proof")?;
    let profile = Profile {
        server: origin.clone(),
        device_id: uuid::Uuid::new_v4(),
        device_token: "agent".into(),
        // Production pairing canonicalizes the root. On Windows the daemon's
        // heartbeat uses the extended canonical path; the identity check must
        // compare the same path representation rather than weakening that check.
        workspace: workspace_path.canonicalize()?,
        state_dir: root.join("runtime"),
        endpoint: endpoint.clone(),
        allow_http_loopback: true,
        allow_write: false,
        allow_process: false,
        allow_terminal: false,
        allow_process_network: false,
        allow_process_outside_workspace: false,
        full_computer: false,
    };
    let binaries = std::env::var_os("RUNTIME_TEST_BIN_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug")
        });
    let daemon = Daemon::ensure(
        &profile,
        &binaries.join(if cfg!(windows) {
            "local-daemon.exe"
        } else {
            "local-daemon"
        }),
    )
    .await?;
    let (workspace, tools) = runner::catalog(&daemon.client).await?;
    conn.execute(
        "INSERT INTO clients VALUES('client','Fixture','[]',?1)",
        params![security::now()],
    )
    .await?;
    conn.execute("INSERT INTO grants(id,family,user_id,client_id,access_hash,refresh_hash,resource,scope,expires_at,refresh_expires_at,selected_device) VALUES('grant','family',?1,'client',?2,'unused',?3,'tools:read tools:execute devices:read',?4,?4,'device')",params![owner.clone(),security::hash("access"),format!("{PUBLIC}/mcp"),security::now()+86400]).await?;
    conn.execute("INSERT INTO devices(id,user_id,token_hash,name,workspace_id,tools,heartbeat_at) VALUES('device',?1,?2,'Fixture',?3,?4,?5)",params![owner,security::hash("agent"),workspace.to_string(),serde_json::to_string(&tools)?,security::now()]).await?;
    let stop = CancellationToken::new();
    let agent = tokio::spawn(runner::run(
        profile,
        daemon.client.clone(),
        root,
        stop.clone(),
    ));
    // Regression: READY_TO_ACTIVATE used to return403 on this empty poll and kill
    // the agent before any first operation could ever reach its computer.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    ensure!(
        !agent.is_finished(),
        "prepaid idle agent stopped before activation"
    );
    let reply=checked(client.post(format!("{origin}/mcp")).bearer_auth("access").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"filesystem.read","arguments":{"path":"proof.txt"}}})).send().await?).await?;
    ensure!(
        reply["result"]["structuredContent"]["result"]
            .to_string()
            .contains("joined commercial proof"),
        "actual remote result missing: {reply}"
    );
    stop.cancel();
    agent.await??;
    daemon.stop().await?;
    webtask.abort();
    Ok(())
}
