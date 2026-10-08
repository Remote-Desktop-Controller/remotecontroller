use clap::Parser;
use remote_service::{CommercialConfig, Config, application};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Remote Desktop Controller: contas, pareamento e MCP remoto"
)]
struct Args {
    #[arg(long, env = "REMOTE_SERVICE_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,
    #[arg(long, env = "REMOTE_SERVICE_PUBLIC_URL")]
    public_url: String,
    #[arg(
        long,
        env = "REMOTE_SERVICE_DATABASE",
        default_value = "remote-service.db"
    )]
    database: PathBuf,
    #[arg(
        long,
        env = "REMOTE_SERVICE_ALLOW_HTTP_LOOPBACK",
        default_value_t = false
    )]
    allow_http_loopback: bool,
    /// Exact callback URLs allowed for OAuth dynamic registration, comma separated.
    #[arg(long, env = "REMOTE_SERVICE_REDIRECT_URIS", value_delimiter = ',')]
    redirect_uris: Vec<String>,
    #[arg(long, env = "REMOTE_SERVICE_COMMERCIAL_PORTAL_URL")]
    commercial_portal_url: Option<String>,
    #[arg(long, env = "REMOTE_SERVICE_COMMERCIAL_SECRET", hide_env_values = true)]
    commercial_secret: Option<String>,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let mut config = Config::new(
        args.public_url,
        args.allow_http_loopback,
        args.redirect_uris
            .into_iter()
            .filter(|uri| !uri.trim().is_empty())
            .collect(),
    )?;
    config.commercial = match (
        args.commercial_portal_url.filter(|v| !v.is_empty()),
        args.commercial_secret.filter(|v| !v.is_empty()),
    ) {
        (None, None) => None,
        (Some(portal), Some(secret)) => Some(CommercialConfig::new(
            portal,
            secret,
            args.allow_http_loopback,
        )?),
        _ => anyhow::bail!("commercial portal URL and secret must be configured together"),
    };
    if args.allow_http_loopback && !args.bind.ip().is_loopback() {
        anyhow::bail!("HTTP de desenvolvimento exige bind loopback");
    }
    if let Some(parent) = args.database.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let app = application(&args.database, config).await?;
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(bind=%args.bind,"remote-service listening; HTTPS terminates at trusted reverse proxy");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
