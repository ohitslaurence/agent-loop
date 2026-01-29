//! loopd - Agent Loop Orchestrator Daemon
//!
//! Main entry point for the daemon binary.
//! See spec: specs/orchestrator-daemon.md

use loopd::{Daemon, DaemonConfig};
use tracing::error;
use tracing_subscriber::{fmt, EnvFilter};

fn main() {
    // Initialize tracing.
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Parse arguments (minimal for now).
    let config = DaemonConfig::default();

    // Run the async main.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    runtime.block_on(async {
        match Daemon::new(config).await {
            Ok(daemon) => {
                // Set up signal handler for graceful shutdown.
                let daemon_ref = &daemon;
                tokio::select! {
                    result = daemon.run() => {
                        if let Err(e) = result {
                            error!("daemon error: {}", e);
                        }
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("received SIGINT");
                        daemon_ref.shutdown();
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
