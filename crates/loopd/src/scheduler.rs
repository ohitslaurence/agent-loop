//! Scheduler module for the orchestrator daemon.
//!
//! Implements run claiming, step enqueuing, and concurrency control.
//! See spec Section 2.1, Section 4.2, Section 5.1.

use loop_core::{Config, Id, QueuePolicy, Run, RunStatus, Step, StepPhase, StepStatus};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};

use crate::storage::{Storage, StorageError};

/// Default maximum concurrent runs (spec says 2-5, defaulting to 3).
pub const DEFAULT_MAX_CONCURRENT_RUNS: usize = 3;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("run not found: {0}")]
    RunNotFound(String),
    #[error("invalid state transition: {0} -> {1}")]
    InvalidTransition(String, String),
    #[error("scheduler shutdown")]
    Shutdown,
}

pub type Result<T> = std::result::Result<T, SchedulerError>;

/// Scheduler state and configuration.
pub struct Scheduler {
    storage: Arc<Storage>,
    /// Semaphore for concurrency limiting (backpressure).
    concurrency_semaphore: Arc<Semaphore>,
    /// Current number of active runs.
    active_runs: AtomicUsize,
    /// Maximum concurrent runs.
    max_concurrent: usize,
    /// Maximum concurrent runs per workspace (optional).
    /// See spec Section 4.2, 5.3: per-workspace cap enforcement.
    max_runs_per_workspace: Option<usize>,
    /// Queue policy: fifo (oldest first) or newest_first.
    /// See spec Section 3.2, 5.3.
    queue_policy: QueuePolicy,
    /// Counter for queue blocked events (per-workspace cap).
    /// See extended spec Section 7.2.
    queue_blocked_workspace: AtomicUsize,
    /// Lock for atomic claim operations.
    claim_lock: Mutex<()>,
    /// Shutdown flag.
    shutdown: std::sync::atomic::AtomicBool,
}

impl Scheduler {
    /// Create a new scheduler with the given storage backend.
    pub fn new(storage: Arc<Storage>, max_concurrent: usize) -> Self {
        Self {
            storage,
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_runs: AtomicUsize::new(0),
            max_concurrent,
            max_runs_per_workspace: None,
            queue_policy: QueuePolicy::Fifo,
            queue_blocked_workspace: AtomicUsize::new(0),
            claim_lock: Mutex::new(()),
            shutdown: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a new scheduler with per-workspace cap.
    ///
    /// See spec Section 4.2, 5.3: per-workspace run caps.
    pub fn new_with_workspace_cap(
        storage: Arc<Storage>,
        max_concurrent: usize,
        max_runs_per_workspace: usize,
    ) -> Self {
        Self {
            storage,
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_runs: AtomicUsize::new(0),
            max_concurrent,
            max_runs_per_workspace: Some(max_runs_per_workspace),
            queue_policy: QueuePolicy::Fifo,
            queue_blocked_workspace: AtomicUsize::new(0),
            claim_lock: Mutex::new(()),
            shutdown: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a new scheduler with queue policy and optional per-workspace cap.
    ///
    /// See spec Section 3.2, 4.2, 5.3: queue policy and per-workspace caps.
    pub fn new_with_policy(
        storage: Arc<Storage>,
        max_concurrent: usize,
        max_runs_per_workspace: Option<usize>,
        queue_policy: QueuePolicy,
    ) -> Self {
        Self {
            storage,
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_runs: AtomicUsize::new(0),
            max_concurrent,
            max_runs_per_workspace,
            queue_policy,
            queue_blocked_workspace: AtomicUsize::new(0),
            claim_lock: Mutex::new(()),
            shutdown: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Create a scheduler with default concurrency settings.
    pub fn with_defaults(storage: Arc<Storage>) -> Self {
        Self::new(storage, DEFAULT_MAX_CONCURRENT_RUNS)
    }

    /// Get the queue blocked workspace counter value.
    ///
    /// See extended spec Section 7.2: metrics.
    pub fn queue_blocked_workspace_count(&self) -> usize {
        self.queue_blocked_workspace.load(Ordering::SeqCst)
    }

    /// Get the current number of active runs.
    pub fn active_run_count(&self) -> usize {
        self.active_runs.load(Ordering::SeqCst)
    }

    /// Get the maximum concurrent runs.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Check if the scheduler can accept more runs.
    pub fn has_capacity(&self) -> bool {
        self.active_run_count() < self.max_concurrent
    }

    /// Signal the scheduler to shut down.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown was signaled.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Claim the next pending run for execution.
    ///
    /// Returns None if:
    /// - No pending runs exist
    /// - Concurrency limit reached (blocks until slot available)
    /// - All pending runs are blocked by per-workspace cap
    /// - Scheduler is shutting down
    ///
    /// On success, transitions the run to RUNNING status.
    ///
    /// Per-workspace cap enforcement (spec Section 4.2, 5.3):
    /// When `max_runs_per_workspace` is set, runs are skipped if their workspace
    /// already has that many RUNNING runs. The run stays PENDING until capacity frees.
    pub async fn claim_next_run(&self) -> Result<Option<Run>> {
        if self.is_shutdown() {
            return Err(SchedulerError::Shutdown);
        }

        // Acquire concurrency permit (blocks if at limit).
        // We try_acquire first to check capacity without blocking.
        let _permit = match self.concurrency_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                // At capacity, wait for a slot (with shutdown check).
                tokio::select! {
                    permit = self.concurrency_semaphore.clone().acquire_owned() => {
                        permit.map_err(|_| SchedulerError::Shutdown)?
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        if self.is_shutdown() {
                            return Err(SchedulerError::Shutdown);
                        }
                        // Re-check after sleep.
                        self.concurrency_semaphore.clone().acquire_owned().await
                            .map_err(|_| SchedulerError::Shutdown)?
                    }
                }
            }
        };

        // Lock to prevent race conditions during claim.
        let _lock = self.claim_lock.lock().await;

        // Find the next pending run based on queue policy (spec Section 5.3).
        // list_runs returns DESC order (newest first).
        let runs = self.storage.list_runs(None).await?;
        let pending_runs: Vec<Run> = runs
            .into_iter()
            .filter(|r| r.status == RunStatus::Pending)
            .collect();

        // Find the first pending run that isn't blocked by workspace cap.
        // Iterate based on queue_policy:
        // - Fifo: reverse order (oldest first)
        // - NewestFirst: forward order (newest first, as returned by list_runs)
        let mut selected_run = None;
        let run_iter: Box<dyn Iterator<Item = &Run>> = match self.queue_policy {
            QueuePolicy::Fifo => Box::new(pending_runs.iter().rev()),
            QueuePolicy::NewestFirst => Box::new(pending_runs.iter()),
        };

        for run in run_iter {
            if let Some(max_per_ws) = self.max_runs_per_workspace {
                let running_in_workspace = self
                    .storage
                    .count_running_runs_for_workspace(&run.workspace_root)
                    .await?;

                if running_in_workspace >= max_per_ws {
                    // Blocked by per-workspace cap (spec Section 6.2).
                    // Log and increment counter (extended spec Section 7.1, 7.2).
                    tracing::info!(
                        run_id = %run.id,
                        workspace = %run.workspace_root,
                        running = running_in_workspace,
                        cap = max_per_ws,
                        "run blocked by per-workspace cap"
                    );
                    self.queue_blocked_workspace.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
            }
            selected_run = Some(run.clone());
            break;
        }

        let Some(run) = selected_run else {
            // No eligible pending runs; release permit by dropping it.
            drop(_permit);
            return Ok(None);
        };

        // Transition to RUNNING.
        self.storage
            .update_run_status(&run.id, RunStatus::Running)
            .await?;
        self.active_runs.fetch_add(1, Ordering::SeqCst);

        // Re-fetch to get updated status.
        let updated_run = self.storage.get_run(&run.id).await?;

        // Permit is intentionally not dropped here; it will be held until
        // release_run is called. We forget it to prevent auto-release.
        std::mem::forget(_permit);

        Ok(Some(updated_run))
    }

    /// Resume runs that were RUNNING when the daemon stopped.
    ///
    /// Called at daemon startup to recover from crashes (Section 5.2).
    pub async fn resume_interrupted_runs(&self) -> Result<Vec<Run>> {
        let runs = self.storage.list_runs(None).await?;
        let running_runs: Vec<Run> = runs
            .into_iter()
            .filter(|r| r.status == RunStatus::Running)
            .collect();

        // Mark each as claimed (acquire permits, update counters).
        for run in &running_runs {
            // Try to acquire permit for each resumed run.
            if let Ok(permit) = self.concurrency_semaphore.clone().try_acquire_owned() {
                self.active_runs.fetch_add(1, Ordering::SeqCst);
                std::mem::forget(permit); // Hold until release.
            } else {
                // Over capacity from crash recovery; mark as paused for later.
                self.storage
                    .update_run_status(&run.id, RunStatus::Paused)
                    .await?;
            }
        }

        // Return only the runs we actually resumed (not paused).
        let resumed = self.storage.list_runs(None).await?;
        Ok(resumed
            .into_iter()
            .filter(|r| r.status == RunStatus::Running)
            .collect())
    }

    /// Enqueue a new step for a run.
    ///
    /// Creates a step record with QUEUED status.
    pub async fn enqueue_step(&self, run_id: &Id, phase: StepPhase) -> Result<Step> {
        // Verify run exists and is in a valid state for new steps.
        let run = self.storage.get_run(run_id).await?;
        if run.status != RunStatus::Running {
            return Err(SchedulerError::InvalidTransition(
                run.status.as_str().to_string(),
                "enqueue_step".to_string(),
            ));
        }

        // Find the highest attempt number for this phase in this run.
        let existing_steps = self.storage.list_steps(run_id).await?;
        let max_attempt = existing_steps
            .iter()
            .filter(|s| s.phase == phase)
            .map(|s| s.attempt)
            .max()
            .unwrap_or(0);

        let step = Step {
            id: Id::new(),
            run_id: run_id.clone(),
            phase,
            status: StepStatus::Queued,
            attempt: max_attempt + 1,
            started_at: None,
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        };

        self.storage.insert_step(&step).await?;
        Ok(step)
    }

    /// Mark a step as in progress.
    pub async fn start_step(&self, step_id: &Id) -> Result<()> {
        let step = self.storage.get_step(step_id).await?;
        if step.status != StepStatus::Queued {
            return Err(SchedulerError::InvalidTransition(
                step.status.as_str().to_string(),
                StepStatus::InProgress.as_str().to_string(),
            ));
        }

        // Update with started_at timestamp.
        // Note: storage.update_step sets ended_at, so we do a direct update here.
        self.storage
            .update_step(step_id, StepStatus::InProgress, None, None)
            .await?;
        Ok(())
    }

    /// Complete a step with result.
    pub async fn complete_step(
        &self,
        step_id: &Id,
        status: StepStatus,
        exit_code: Option<i32>,
        output_path: Option<&str>,
    ) -> Result<()> {
        let step = self.storage.get_step(step_id).await?;
        if step.status != StepStatus::InProgress {
            return Err(SchedulerError::InvalidTransition(
                step.status.as_str().to_string(),
                status.as_str().to_string(),
            ));
        }

        self.storage
            .update_step(step_id, status, exit_code, output_path)
            .await?;
        Ok(())
    }

    /// Release a run after completion or failure.
    ///
    /// Updates status and releases concurrency permit.
    pub async fn release_run(&self, run_id: &Id, status: RunStatus) -> Result<()> {
        let run = self.storage.get_run(run_id).await?;

        // Only release if currently RUNNING.
        if run.status != RunStatus::Running {
            return Err(SchedulerError::InvalidTransition(
                run.status.as_str().to_string(),
                status.as_str().to_string(),
            ));
        }

        // Update status.
        self.storage.update_run_status(run_id, status).await?;

        // Release concurrency slot.
        let prev = self.active_runs.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            // Add permit back to semaphore.
            self.concurrency_semaphore.add_permits(1);
        }

        Ok(())
    }

    /// Pause a running run.
    pub async fn pause_run(&self, run_id: &Id) -> Result<()> {
        self.release_run(run_id, RunStatus::Paused).await
    }

    /// Resume a paused run.
    ///
    /// Returns the run if successfully resumed, None if no capacity.
    pub async fn resume_run(&self, run_id: &Id) -> Result<Option<Run>> {
        let run = self.storage.get_run(run_id).await?;
        if run.status != RunStatus::Paused {
            return Err(SchedulerError::InvalidTransition(
                run.status.as_str().to_string(),
                RunStatus::Running.as_str().to_string(),
            ));
        }

        // Try to acquire a concurrency slot.
        let permit = match self.concurrency_semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Ok(None), // No capacity.
        };

        self.storage
            .update_run_status(run_id, RunStatus::Running)
            .await?;
        self.active_runs.fetch_add(1, Ordering::SeqCst);
        std::mem::forget(permit);

        let updated = self.storage.get_run(run_id).await?;
        Ok(Some(updated))
    }

    /// Cancel a run (from any state except COMPLETED).
    pub async fn cancel_run(&self, run_id: &Id) -> Result<()> {
        let run = self.storage.get_run(run_id).await?;

        match run.status {
            RunStatus::Completed => {
                return Err(SchedulerError::InvalidTransition(
                    run.status.as_str().to_string(),
                    RunStatus::Canceled.as_str().to_string(),
                ));
            }
            RunStatus::Running => {
                // Release the concurrency slot.
                self.storage
                    .update_run_status(run_id, RunStatus::Canceled)
                    .await?;
                let prev = self.active_runs.fetch_sub(1, Ordering::SeqCst);
                if prev > 0 {
                    self.concurrency_semaphore.add_permits(1);
                }
            }
            _ => {
                self.storage
                    .update_run_status(run_id, RunStatus::Canceled)
                    .await?;
            }
        }

        Ok(())
    }

    /// Get the next step to execute for a run.
    ///
    /// Returns the first QUEUED step, or None if no steps are queued.
    pub async fn get_next_step(&self, run_id: &Id) -> Result<Option<Step>> {
        let steps = self.storage.list_steps(run_id).await?;
        Ok(steps.into_iter().find(|s| s.status == StepStatus::Queued))
    }

    /// Determine the next phase for a run based on current state.
    ///
    /// Implements the main flow from Section 5.1:
    /// implementation -> review -> verification -> (watchdog if signals) -> completion
    ///
    /// When `reviewer=true` (default): implementation -> review -> verification
    /// When `reviewer=false`: implementation -> verification (skip review)
    ///
    /// Verification failure handling (Section 5.2):
    /// When verification fails, we requeue implementation (do not advance plan).
    /// Runner notes are written by the verifier module.
    pub async fn determine_next_phase(&self, run_id: &Id) -> Result<Option<StepPhase>> {
        let run = self.storage.get_run(run_id).await?;
        let steps = self.storage.list_steps(run_id).await?;

        // Check if reviewer is enabled (defaults to true per spec Section 4.1).
        let reviewer_enabled = Self::is_reviewer_enabled(&run);

        // Check for the most recent step (by creation order, not just succeeded).
        // This handles verification failure: if the last step was a failed verification,
        // we need to requeue implementation per spec Section 5.2.
        let last_step = steps.last();

        if let Some(step) = last_step {
            // If the last step was a failed verification, requeue implementation.
            // Per spec Section 5.2: "Verification fails: write runner notes,
            // requeue implementation step, do not advance plan."
            if step.phase == StepPhase::Verification && step.status == StepStatus::Failed {
                return Ok(Some(StepPhase::Implementation));
            }
        }

        // Find the last completed step.
        let last_succeeded = steps
            .iter()
            .filter(|s| s.status == StepStatus::Succeeded)
            .last();

        match last_succeeded {
            None => {
                // No steps completed yet; start with implementation.
                Ok(Some(StepPhase::Implementation))
            }
            Some(step) => {
                // Determine next phase based on last completed.
                match step.phase {
                    StepPhase::Implementation => {
                        // Skip review if reviewer is disabled.
                        if reviewer_enabled {
                            Ok(Some(StepPhase::Review))
                        } else {
                            Ok(Some(StepPhase::Verification))
                        }
                    }
                    StepPhase::Review => Ok(Some(StepPhase::Verification)),
                    StepPhase::Verification => {
                        // Verification passed; loop back to implementation or complete.
                        // Completion detection is handled by the runner.
                        Ok(Some(StepPhase::Implementation))
                    }
                    StepPhase::Watchdog => {
                        // After watchdog, retry implementation.
                        Ok(Some(StepPhase::Implementation))
                    }
                    StepPhase::Merge => {
                        // Merge is terminal.
                        Ok(None)
                    }
                }
            }
        }
    }

    /// Check if reviewer is enabled for a run.
    ///
    /// Parses the run's config_json to check the `reviewer` field.
    /// Defaults to true per spec Section 4.1 (config.rs default).
    fn is_reviewer_enabled(run: &Run) -> bool {
        if let Some(config_json) = &run.config_json {
            // Try to parse as full Config or just extract reviewer field.
            if let Ok(config) = serde_json::from_str::<Config>(config_json) {
                return config.reviewer;
            }
            // Fallback: try to parse as a partial JSON object with just reviewer.
            if let Ok(obj) = serde_json::from_str::<serde_json::Value>(config_json) {
                if let Some(reviewer) = obj.get("reviewer").and_then(|v| v.as_bool()) {
                    return reviewer;
                }
            }
        }
        // Default: reviewer enabled.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use loop_core::RunNameSource;
    use tempfile::TempDir;

    struct TestScheduler {
        scheduler: Scheduler,
        _dir: TempDir,
    }

    async fn create_test_scheduler() -> TestScheduler {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        let storage = Arc::new(storage);
        let scheduler = Scheduler::new(storage, 2);
        TestScheduler {
            scheduler,
            _dir: dir,
        }
    }

    fn create_test_run(id: &str) -> Run {
        let now = Utc::now();
        Run {
            id: Id::from_string(id),
            name: format!("test-run-{}", id),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn claim_next_run_returns_none_when_empty() {
        let ts = create_test_scheduler().await;
        let result = ts.scheduler.claim_next_run().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn claim_next_run_transitions_to_running() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();

        let claimed = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed.is_some());

        let claimed = claimed.unwrap();
        assert_eq!(claimed.status, RunStatus::Running);
        assert_eq!(ts.scheduler.active_run_count(), 1);
    }

    #[tokio::test]
    async fn respects_concurrency_limit() {
        let ts = create_test_scheduler().await;

        // Insert 3 runs but limit is 2.
        for i in 0..3 {
            let run = create_test_run(&format!("run-{}", i));
            ts.scheduler.storage.insert_run(&run).await.unwrap();
        }

        // Claim first two.
        ts.scheduler.claim_next_run().await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        assert_eq!(ts.scheduler.active_run_count(), 2);
        assert!(!ts.scheduler.has_capacity());
    }

    #[tokio::test]
    async fn enqueue_step_creates_queued_step() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();

        // Claim the run first (must be RUNNING to enqueue steps).
        ts.scheduler.claim_next_run().await.unwrap();

        let step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();

        assert_eq!(step.phase, StepPhase::Implementation);
        assert_eq!(step.status, StepStatus::Queued);
        assert_eq!(step.attempt, 1);
    }

    #[tokio::test]
    async fn release_run_frees_capacity() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();

        ts.scheduler.claim_next_run().await.unwrap();
        assert_eq!(ts.scheduler.active_run_count(), 1);

        ts.scheduler
            .release_run(&run.id, RunStatus::Completed)
            .await
            .unwrap();
        assert_eq!(ts.scheduler.active_run_count(), 0);
        assert!(ts.scheduler.has_capacity());
    }

    #[tokio::test]
    async fn determine_next_phase_starts_with_implementation() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Implementation));
    }

    #[test]
    fn is_reviewer_enabled_defaults_to_true() {
        let run = create_test_run("run-1");
        assert!(Scheduler::is_reviewer_enabled(&run));
    }

    #[test]
    fn is_reviewer_enabled_respects_config_json() {
        let now = Utc::now();
        let mut run = Run {
            id: Id::from_string("run-1"),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            config_json: Some(r#"{"reviewer": false}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        assert!(!Scheduler::is_reviewer_enabled(&run));

        // Also test with true
        run.config_json = Some(r#"{"reviewer": true}"#.to_string());
        assert!(Scheduler::is_reviewer_enabled(&run));
    }

    #[tokio::test]
    async fn determine_next_phase_goes_to_review_when_enabled() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Enqueue and complete implementation step.
        let step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();
        ts.scheduler.start_step(&step.id).await.unwrap();
        ts.scheduler
            .complete_step(&step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Next phase should be Review (reviewer=true by default).
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Review));
    }

    #[tokio::test]
    async fn determine_next_phase_skips_review_when_disabled() {
        let ts = create_test_scheduler().await;
        let now = Utc::now();
        let run = Run {
            id: Id::from_string("run-no-review"),
            name: "test-run-no-review".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: None,
            config_json: Some(r#"{"reviewer": false}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Enqueue and complete implementation step.
        let step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();
        ts.scheduler.start_step(&step.id).await.unwrap();
        ts.scheduler
            .complete_step(&step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Next phase should be Verification (skip review).
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Verification));
    }

    #[tokio::test]
    async fn determine_next_phase_review_to_verification() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Complete implementation then review.
        let impl_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();
        ts.scheduler.start_step(&impl_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&impl_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        let review_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Review)
            .await
            .unwrap();
        ts.scheduler.start_step(&review_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&review_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Next phase should be Verification.
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Verification));
    }

    #[tokio::test]
    async fn determine_next_phase_verification_failure_requeues_implementation() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Complete implementation step.
        let impl_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();
        ts.scheduler.start_step(&impl_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&impl_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Complete review step.
        let review_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Review)
            .await
            .unwrap();
        ts.scheduler.start_step(&review_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&review_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Fail verification step (spec Section 5.2).
        let verify_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Verification)
            .await
            .unwrap();
        ts.scheduler.start_step(&verify_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&verify_step.id, StepStatus::Failed, Some(1), None)
            .await
            .unwrap();

        // Next phase should be Implementation (requeue, do not advance plan).
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Implementation));
    }

    #[tokio::test]
    async fn determine_next_phase_verification_success_continues() {
        let ts = create_test_scheduler().await;
        let run = create_test_run("run-1");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Complete implementation step.
        let impl_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Implementation)
            .await
            .unwrap();
        ts.scheduler.start_step(&impl_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&impl_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Complete review step.
        let review_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Review)
            .await
            .unwrap();
        ts.scheduler.start_step(&review_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&review_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Pass verification step.
        let verify_step = ts
            .scheduler
            .enqueue_step(&run.id, StepPhase::Verification)
            .await
            .unwrap();
        ts.scheduler.start_step(&verify_step.id).await.unwrap();
        ts.scheduler
            .complete_step(&verify_step.id, StepStatus::Succeeded, Some(0), None)
            .await
            .unwrap();

        // Next phase should be Implementation (continue to next iteration).
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Implementation));
    }

    // --- Per-workspace cap tests (spec Section 4.2, 5.3) ---

    async fn create_test_scheduler_with_workspace_cap(cap: usize) -> TestScheduler {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        let storage = Arc::new(storage);
        let scheduler = Scheduler::new_with_workspace_cap(storage, 5, cap);
        TestScheduler {
            scheduler,
            _dir: dir,
        }
    }

    fn create_test_run_in_workspace(id: &str, workspace: &str) -> Run {
        let now = Utc::now();
        Run {
            id: Id::from_string(id),
            name: format!("test-run-{}", id),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: workspace.to_string(),
            spec_path: format!("{}/spec.md", workspace),
            plan_path: Some(format!("{}/plan.md", workspace)),
            worktree: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn workspace_cap_blocks_run_when_at_limit() {
        let ts = create_test_scheduler_with_workspace_cap(1).await;

        // Insert two runs in the same workspace.
        let run1 = create_test_run_in_workspace("run-1", "/workspace-a");
        let run2 = create_test_run_in_workspace("run-2", "/workspace-a");

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();

        // Claim first run - should succeed.
        let claimed1 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed1.is_some());
        assert_eq!(claimed1.unwrap().id, run1.id);

        // Claim second run - should return None (blocked by workspace cap).
        let claimed2 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed2.is_none());

        // Counter should have been incremented.
        assert!(ts.scheduler.queue_blocked_workspace_count() > 0);
    }

    #[tokio::test]
    async fn workspace_cap_allows_run_from_different_workspace() {
        let ts = create_test_scheduler_with_workspace_cap(1).await;

        // Insert runs in different workspaces.
        let run1 = create_test_run_in_workspace("run-1", "/workspace-a");
        let run2 = create_test_run_in_workspace("run-2", "/workspace-b");

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();

        // Claim first run from workspace-a.
        let claimed1 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed1.is_some());
        assert_eq!(claimed1.unwrap().workspace_root, "/workspace-a");

        // Claim second run - should succeed (different workspace).
        let claimed2 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed2.is_some());
        assert_eq!(claimed2.unwrap().workspace_root, "/workspace-b");
    }

    #[tokio::test]
    async fn workspace_cap_releases_slot_when_run_completes() {
        let ts = create_test_scheduler_with_workspace_cap(1).await;

        // Insert two runs in the same workspace.
        let run1 = create_test_run_in_workspace("run-1", "/workspace-a");
        let run2 = create_test_run_in_workspace("run-2", "/workspace-a");

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();

        // Claim first run.
        let claimed1 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed1.is_some());

        // Second claim blocked.
        let blocked = ts.scheduler.claim_next_run().await.unwrap();
        assert!(blocked.is_none());

        // Release first run.
        ts.scheduler
            .release_run(&run1.id, RunStatus::Completed)
            .await
            .unwrap();

        // Now second run should be claimable.
        let claimed2 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed2.is_some());
        assert_eq!(claimed2.unwrap().id, run2.id);
    }

    #[tokio::test]
    async fn no_workspace_cap_allows_multiple_runs_same_workspace() {
        // Standard scheduler without workspace cap.
        let ts = create_test_scheduler().await;

        // Insert two runs in the same workspace.
        let run1 = create_test_run("run-1");
        let run2 = create_test_run("run-2");

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();

        // Both should be claimable (no workspace cap).
        let claimed1 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed1.is_some());

        let claimed2 = ts.scheduler.claim_next_run().await.unwrap();
        assert!(claimed2.is_some());
    }

    // --- Queue policy tests (spec Section 3.2, 5.3) ---

    async fn create_test_scheduler_with_policy(policy: QueuePolicy) -> TestScheduler {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        let storage = Arc::new(storage);
        let scheduler = Scheduler::new_with_policy(storage, 5, None, policy);
        TestScheduler {
            scheduler,
            _dir: dir,
        }
    }

    fn create_test_run_with_order(id: &str, order: i64) -> Run {
        // Use a controlled timestamp to ensure ordering.
        let now = chrono::DateTime::from_timestamp(1700000000 + order, 0)
            .unwrap()
            .with_timezone(&Utc);
        Run {
            id: Id::from_string(id),
            name: format!("test-run-{}", id),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn queue_policy_fifo_claims_oldest_first() {
        let ts = create_test_scheduler_with_policy(QueuePolicy::Fifo).await;

        // Insert runs with explicit ordering (run-1 is oldest).
        let run1 = create_test_run_with_order("run-1", 0);
        let run2 = create_test_run_with_order("run-2", 100);
        let run3 = create_test_run_with_order("run-3", 200);

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();
        ts.scheduler.storage.insert_run(&run3).await.unwrap();

        // With FIFO policy, oldest run (run-1) should be claimed first.
        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run1.id, "FIFO should claim oldest run first");

        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run2.id);

        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run3.id);
    }

    #[tokio::test]
    async fn queue_policy_newest_first_claims_newest_first() {
        let ts = create_test_scheduler_with_policy(QueuePolicy::NewestFirst).await;

        // Insert runs with explicit ordering (run-3 is newest).
        let run1 = create_test_run_with_order("run-1", 0);
        let run2 = create_test_run_with_order("run-2", 100);
        let run3 = create_test_run_with_order("run-3", 200);

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();
        ts.scheduler.storage.insert_run(&run3).await.unwrap();

        // With NewestFirst policy, newest run (run-3) should be claimed first.
        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(
            claimed.id, run3.id,
            "NewestFirst should claim newest run first"
        );

        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run2.id);

        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run1.id);
    }

    #[tokio::test]
    async fn default_policy_is_fifo() {
        // Standard scheduler should use FIFO (default).
        let ts = create_test_scheduler().await;

        let run1 = create_test_run_with_order("run-1", 0);
        let run2 = create_test_run_with_order("run-2", 100);

        ts.scheduler.storage.insert_run(&run1).await.unwrap();
        ts.scheduler.storage.insert_run(&run2).await.unwrap();

        // Default should be FIFO: oldest run claimed first.
        let claimed = ts.scheduler.claim_next_run().await.unwrap().unwrap();
        assert_eq!(claimed.id, run1.id, "Default policy should be FIFO");
    }

    // --- Runner pipeline progression tests (spec Section 5.1) ---

    /// Helper to advance through a phase: enqueue, start, complete with given status.
    async fn advance_phase(
        scheduler: &Scheduler,
        run_id: &Id,
        phase: StepPhase,
        status: StepStatus,
        exit_code: i32,
    ) -> Step {
        let step = scheduler.enqueue_step(run_id, phase).await.unwrap();
        scheduler.start_step(&step.id).await.unwrap();
        scheduler
            .complete_step(&step.id, status, Some(exit_code), None)
            .await
            .unwrap();
        step
    }

    #[tokio::test]
    async fn pipeline_progression_full_cycle_with_reviewer() {
        // Test spec Section 5.1 main flow with reviewer=true (default):
        // implementation -> review -> verification -> implementation (next iteration)
        let ts = create_test_scheduler().await;
        let run = create_test_run("pipeline-run");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Phase 1: Should start with Implementation
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Implementation),
            "Pipeline should start with Implementation"
        );
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Implementation,
            StepStatus::Succeeded,
            0,
        )
        .await;

        // Phase 2: After Implementation success -> Review (reviewer=true by default)
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Review),
            "After Implementation -> Review"
        );
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Review,
            StepStatus::Succeeded,
            0,
        )
        .await;

        // Phase 3: After Review success -> Verification
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Verification),
            "After Review -> Verification"
        );
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Verification,
            StepStatus::Succeeded,
            0,
        )
        .await;

        // Phase 4: After Verification success -> Implementation (next iteration)
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Implementation),
            "After Verification success -> Implementation (next iteration)"
        );
    }

    #[tokio::test]
    async fn pipeline_progression_verification_failure_requeues_implementation() {
        // Test spec Section 5.2: verification failure requeues implementation
        let ts = create_test_scheduler().await;
        let run = create_test_run("pipeline-verify-fail");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Complete implementation -> review -> verification (FAIL)
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Implementation,
            StepStatus::Succeeded,
            0,
        )
        .await;
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Review,
            StepStatus::Succeeded,
            0,
        )
        .await;
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Verification,
            StepStatus::Failed,
            1,
        )
        .await;

        // After Verification failure -> Implementation (requeue, not advance)
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Implementation),
            "Verification failure should requeue Implementation"
        );
    }

    #[tokio::test]
    async fn pipeline_progression_without_reviewer() {
        // Test spec Section 5.1 flow with reviewer=false:
        // implementation -> verification (skip review)
        let ts = create_test_scheduler().await;
        let now = Utc::now();
        let run = Run {
            id: Id::from_string("no-reviewer-run"),
            name: "no-reviewer-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: None,
            config_json: Some(r#"{"reviewer": false}"#.to_string()),
            worktree: None,
            created_at: now,
            updated_at: now,
        };
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        // Phase 1: Implementation
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(phase, Some(StepPhase::Implementation));
        advance_phase(
            &ts.scheduler,
            &run.id,
            StepPhase::Implementation,
            StepStatus::Succeeded,
            0,
        )
        .await;

        // Phase 2: Should skip Review and go directly to Verification
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Verification),
            "With reviewer=false, should skip Review -> Verification"
        );
    }

    #[tokio::test]
    async fn pipeline_progression_multiple_iterations() {
        // Test multiple complete iterations through the pipeline
        let ts = create_test_scheduler().await;
        let run = create_test_run("multi-iter-run");
        ts.scheduler.storage.insert_run(&run).await.unwrap();
        ts.scheduler.claim_next_run().await.unwrap();

        for iteration in 1..=3 {
            // Implementation
            let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
            assert_eq!(
                phase,
                Some(StepPhase::Implementation),
                "Iteration {}: should be at Implementation",
                iteration
            );
            advance_phase(
                &ts.scheduler,
                &run.id,
                StepPhase::Implementation,
                StepStatus::Succeeded,
                0,
            )
            .await;

            // Review
            let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
            assert_eq!(phase, Some(StepPhase::Review));
            advance_phase(
                &ts.scheduler,
                &run.id,
                StepPhase::Review,
                StepStatus::Succeeded,
                0,
            )
            .await;

            // Verification
            let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
            assert_eq!(phase, Some(StepPhase::Verification));
            advance_phase(
                &ts.scheduler,
                &run.id,
                StepPhase::Verification,
                StepStatus::Succeeded,
                0,
            )
            .await;
        }

        // After 3 iterations, next phase should still be Implementation
        let phase = ts.scheduler.determine_next_phase(&run.id).await.unwrap();
        assert_eq!(
            phase,
            Some(StepPhase::Implementation),
            "After multiple iterations, pipeline should continue"
        );
    }
}
