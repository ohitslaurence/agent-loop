//! loopd - Agent Loop Orchestrator Daemon
//!
//! Main entry point for the daemon process.
//! See spec: specs/orchestrator-daemon.md

pub mod naming;
pub mod runner;
pub mod scheduler;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;

use scheduler::Scheduler;
use storage::Storage;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

/// Daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Path to the SQLite database.
    pub db_path: PathBuf,
    /// Maximum concurrent runs (default: 3).
    pub max_concurrent_runs: usize,
    /// HTTP server port (default: 7700).
    pub port: u16,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            max_concurrent_runs: scheduler::DEFAULT_MAX_CONCURRENT_RUNS,
            port: 7700,
        }
    }
}

/// Get the default database path (~/.local/share/loopd/loopd.db).
fn default_db_path() -> PathBuf {
    let data_dir = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/share")
        });
    data_dir.join("loopd").join("loopd.db")
}

/// Daemon state.
pub struct Daemon {
    config: DaemonConfig,
    storage: Arc<Storage>,
    scheduler: Arc<Scheduler>,
}

impl Daemon {
    /// Create a new daemon with the given configuration.
    pub async fn new(config: DaemonConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let storage = Storage::new(&config.db_path).await?;
        storage.migrate_embedded().await?;
        let storage = Arc::new(storage);

        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&storage),
            config.max_concurrent_runs,
        ));

        Ok(Self {
            config,
            storage,
            scheduler,
        })
    }

    /// Get a reference to the storage backend.
    pub fn storage(&self) -> &Arc<Storage> {
        &self.storage
    }

    /// Get a reference to the scheduler.
    pub fn scheduler(&self) -> &Arc<Scheduler> {
        &self.scheduler
    }

    /// Run the daemon main loop.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("loopd starting on port {}", self.config.port);
        info!("database: {}", self.config.db_path.display());
        info!("max concurrent runs: {}", self.config.max_concurrent_runs);

        // Resume any runs that were interrupted by a previous crash.
        match self.scheduler.resume_interrupted_runs().await {
            Ok(resumed) => {
                if !resumed.is_empty() {
                    info!("resumed {} interrupted run(s)", resumed.len());
                    for run in &resumed {
                        info!("  - {} ({})", run.name, run.id);
                    }
                }
            }
            Err(e) => {
                warn!("failed to resume interrupted runs: {}", e);
            }
        }

        // Main scheduling loop.
        loop {
            if self.scheduler.is_shutdown() {
                info!("shutdown signal received, exiting");
                break;
            }

            // Try to claim the next pending run.
            match self.scheduler.claim_next_run().await {
                Ok(Some(run)) => {
                    info!("claimed run: {} ({})", run.name, run.id);

                    // Spawn a task to process this run.
                    let scheduler = Arc::clone(&self.scheduler);
                    let storage = Arc::clone(&self.storage);
                    tokio::spawn(async move {
                        if let Err(e) = process_run(scheduler, storage, run).await {
                            error!("run processing failed: {}", e);
                        }
                    });
                }
                Ok(None) => {
                    // No pending runs; sleep before checking again.
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                }
                Err(scheduler::SchedulerError::Shutdown) => {
                    info!("scheduler shutdown");
                    break;
                }
                Err(e) => {
                    error!("scheduler error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }

        Ok(())
    }

    /// Signal the daemon to shut down.
    pub fn shutdown(&self) {
        info!("shutdown requested");
        self.scheduler.shutdown();
    }
}

/// Process a single run through all phases.
async fn process_run(
    scheduler: Arc<Scheduler>,
    _storage: Arc<Storage>,
    run: loop_core::Run,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("processing run: {} ({})", run.name, run.id);

    // Main phase loop.
    loop {
        // Determine the next phase.
        let next_phase = scheduler.determine_next_phase(&run.id).await?;

        let Some(phase) = next_phase else {
            // No more phases; run is complete.
            info!("run complete: {}", run.id);
            scheduler
                .release_run(&run.id, loop_core::RunStatus::Completed)
                .await?;
            break;
        };

        // Enqueue and execute the step.
        let step = scheduler.enqueue_step(&run.id, phase).await?;
        info!(
            "executing step: {} phase={:?} attempt={}",
            step.id, step.phase, step.attempt
        );

        scheduler.start_step(&step.id).await?;

        // TODO: Execute the actual phase logic via the runner module.
        // For now, mark as succeeded to allow the loop to progress.
        scheduler
            .complete_step(&step.id, loop_core::StepStatus::Succeeded, Some(0), None)
            .await?;

        // TODO: Check for completion token in output and break if detected.
        // For now, break after one iteration to prevent infinite loop.
        warn!("runner not yet implemented, stopping after first step");
        scheduler
            .release_run(&run.id, loop_core::RunStatus::Paused)
            .await?;
        break;
    }

    Ok(())
}

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
                        info!("received SIGINT");
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
