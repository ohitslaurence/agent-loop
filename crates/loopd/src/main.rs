//! loopd - Agent Loop Orchestrator Daemon
//!
//! Main entry point for the daemon binary.
//! See spec: specs/orchestrator-daemon.md

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use clap::Parser;
use loopd::{Daemon, DaemonConfig};
use tracing::error;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser)]
#[command(name = "loopd", about = "Agent Loop Orchestrator Daemon", version)]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value = "7700")]
    port: u16,
}

fn main() {
    let cli = Cli::parse();

    // Initialize tracing.
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = DaemonConfig {
        port: cli.port,
        ..Default::default()
    };

    // Run the async main.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    runtime.block_on(async {
        match Daemon::new(config).await {
            Ok(daemon) => {
                // Set up signal handlers for graceful shutdown.
                let daemon_ref = &daemon;

                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sigterm = signal(SignalKind::terminate())
                        .expect("failed to register SIGTERM handler");
                    let mut sigint =
                        signal(SignalKind::interrupt()).expect("failed to register SIGINT handler");

                    tokio::select! {
                        result = daemon.run() => {
                            if let Err(e) = result {
                                error!("daemon error: {}", e);
                            }
                        }
                        _ = sigint.recv() => {
                            tracing::info!("received SIGINT, initiating graceful shutdown");
                            daemon_ref.shutdown();
                        }
                        _ = sigterm.recv() => {
                            tracing::info!("received SIGTERM, initiating graceful shutdown");
                            daemon_ref.shutdown();
                        }
                    }
                }

                #[cfg(not(unix))]
                {
                    tokio::select! {
                        result = daemon.run() => {
                            if let Err(e) = result {
                                error!("daemon error: {}", e);
                            }
                        }
                        _ = tokio::signal::ctrl_c() => {
                            tracing::info!("received SIGINT, initiating graceful shutdown");
                            daemon_ref.shutdown();
                        }
                    }
                }
            }
            Err(e) => {
                error!("failed to initialize daemon: {}", e);
                std::process::exit(1);
            }
        }
    });
}
