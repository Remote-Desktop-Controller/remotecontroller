//! `rdc-monitor`: Real-time interactive visual telemetry viewer for Remote Desktop Controller.
//!
//! Connects to the local daemon IPC channel (`\\.\pipe\rdc-telemetry`) and displays
//! an unbuffered, colorized stream of AI activity including tool invocations, commands,
//! diffs, and execution latencies.

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "rdc-monitor",
    version,
    about = "Real-time interactive AI observability monitor for Remote Desktop Controller"
)]
struct Args {
    #[arg(
        long,
        default_value = r"\\.\pipe\rdc-telemetry",
        help = "Named pipe endpoint to connect to"
    )]
    pipe: String,

    #[arg(long, help = "Disable ANSI color formatting in output")]
    no_color: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let use_ansi = !args.no_color;

    render_header(&args.pipe, use_ansi);

    #[cfg(windows)]
    {
        use runtime_infrastructure::observer::server::connect_telemetry_client;

        let mut rx = None;
        for attempt in 1..=10 {
            match connect_telemetry_client(&args.pipe).await {
                Ok(client_rx) => {
                    rx = Some(client_rx);
                    break;
                }
                Err(_) => {
                    if attempt == 1 {
                        println!("Daemon not ready yet, waiting for pipe '{}'...", args.pipe);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
            }
        }

        let mut rx = rx.context(format!(
            "Failed to connect to Named Pipe at '{}'. Ensure the RDC daemon is running.",
            args.pipe
        ))?;

        println!("Connected. Awaiting events... (Press Ctrl+C to exit)\n");

        while let Some(event) = rx.recv().await {
            println!("{}", event.format_display(use_ansi));
        }

        println!("\nTelemetry connection closed by host daemon.");
    }

    #[cfg(not(windows))]
    {
        println!("Note: Named Pipe telemetry is supported on Windows.");
    }

    Ok(())
}

fn render_header(pipe: &str, use_ansi: bool) {
    if use_ansi {
        println!("\x1b[1;34m╔══════════════════════════════════════════════════════════════════════════════╗\x1b[0m");
        println!("\x1b[1;34m║\x1b[1;37m        REMOTE DESKTOP CONTROLLER — REAL-TIME AI AUDIT MONITOR               \x1b[1;34m║\x1b[0m");
        println!("\x1b[1;34m╚══════════════════════════════════════════════════════════════════════════════╝\x1b[0m");
        println!("\x1b[90mTarget Pipe:\x1b[0m \x1b[33m{pipe}\x1b[0m\n");
    } else {
        println!("================================================================================");
        println!("        REMOTE DESKTOP CONTROLLER — REAL-TIME AI AUDIT MONITOR                  ");
        println!("================================================================================");
        println!("Target Pipe: {pipe}\n");
    }
}
