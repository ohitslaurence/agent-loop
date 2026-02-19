//! loopd - Agent Loop Orchestrator Daemon
//!
//! Library components for the daemon process.
//! See spec: specs/orchestrator-daemon.md

pub mod git;
pub mod handlers;
pub mod naming;
pub mod postmortem;
pub mod runner;
pub mod scheduler;
pub mod server;
pub mod skills;
pub mod storage;
pub mod verifier;
pub mod watchdog;
pub mod worktree;
pub mod worktree_worktrunk;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Grace period for in-flight steps to abort during shutdown.
///
/// After signaling cancellation, the daemon waits this long before
/// force-terminating the HTTP server. Steps use this time to clean up
/// and write partial results.
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Poll interval when no pending runs are available.
///
/// The main loop sleeps this long between checks for new work.
/// Lower values increase responsiveness but add CPU overhead.
const CLAIM_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Backoff after scheduler errors before retrying.
const SCHEDULER_ERROR_BACKOFF: Duration = Duration::from_secs(1);
const MAX_REVIEW_SNAPSHOT_BYTES: usize = 50 * 1024 * 1024;

use crate::handlers::review::build_run_diff_snapshot;
use chrono::Utc;
use loop_core::completion::check_completion;
use loop_core::events::{
    EventPayload, PostmortemEndPayload, PostmortemStartPayload, RunCompletedPayload,
    RunFailedPayload, SelectedSkillPayload, SkillsDiscoveredPayload, SkillsLoadFailedPayload,
    SkillsSelectedPayload, SkillsTruncatedPayload, StepFinishedPayload, StepStartedPayload,
    WatchdogRewritePayload, WorktreeCreatedPayload, WorktreeProviderSelectedPayload,
    WorktreeRemovedPayload,
};
use loop_core::plan::{select_task, TaskSelection};
use loop_core::skills::SkillMetadata;
use loop_core::types::{MergeStrategy, WorktreeProvider};
use loop_core::{
    mirror_artifact, write_and_mirror_artifact, Artifact, Config, Id, ReviewStatus, Run, StepPhase,
    StepStatus,
};
use postmortem::ExitReason;
use runner::{Runner, RunnerConfig, RunnerError};
use scheduler::Scheduler;
use skills::{
    load_skill_body, render_available_skills, select_skills, LoadFailureEvent, SkillSelection,
    SkillsMetrics, StepKind, TruncationEvent,
};
use storage::Storage;
use tracing::{error, info, warn};
use uuid::Uuid;
use verifier::{Verifier, VerifierConfig};
use watchdog::{Watchdog, WatchdogAction};

/// Type alias for application-level errors with context and backtraces.
pub type AppResult<T> = eyre::Result<T>;

// --- Consecutive Failure Detection (consecutive-failure-detection.md) ---

/// Per-phase consecutive failure counters.
///
/// Tracks consecutive failures for verification and review phases.
/// A success resets the counter to 0; a failure increments it.
/// See spec Section 3.3 (Derived State).
#[derive(Debug, Default)]
struct ConsecutiveFailures {
    verification: u32,
    review: u32,
}

impl ConsecutiveFailures {
    /// Compute consecutive failure counters from step history.
    ///
    /// Iterates through steps in chronological order, incrementing counters
    /// on failure and resetting on success. Only counts Review and Verification phases.
    /// See spec Section 3.3 and 5.1.
    fn from_steps(steps: &[loop_core::Step]) -> Self {
        let mut counters = Self::default();
        for step in steps {
            match step.phase {
                StepPhase::Verification => {
                    if step.status == StepStatus::Failed {
                        counters.verification += 1;
                    } else if step.status == StepStatus::Succeeded {
                        counters.verification = 0;
                    }
                }
                StepPhase::Review => {
                    if step.status == StepStatus::Failed {
                        counters.review += 1;
                    } else if step.status == StepStatus::Succeeded {
                        counters.review = 0;
                    }
                }
                _ => {}
            }
        }
        counters
    }

    /// Update counters after a step completion.
    ///
    /// Increments the counter for the phase on failure, resets on success.
    fn update(&mut self, phase: StepPhase, status: StepStatus) {
        match phase {
            StepPhase::Verification => {
                if status == StepStatus::Failed {
                    self.verification += 1;
                } else if status == StepStatus::Succeeded {
                    self.verification = 0;
                }
            }
            StepPhase::Review => {
                if status == StepStatus::Failed {
                    self.review += 1;
                } else if status == StepStatus::Succeeded {
                    self.review = 0;
                }
            }
            _ => {}
        }
    }

    /// Check if any threshold is exceeded.
    ///
    /// Returns `Some((phase, count, limit))` if a threshold is breached, `None` otherwise.
    /// A limit of 0 disables the check for that phase.
    /// See spec Section 5.1.
    fn check_thresholds(&self, config: &Config) -> Option<(StepPhase, u32, u32)> {
        if config.max_consecutive_verification_failures > 0
            && self.verification >= config.max_consecutive_verification_failures
        {
            return Some((
                StepPhase::Verification,
                self.verification,
                config.max_consecutive_verification_failures,
            ));
        }
        if config.max_consecutive_review_failures > 0
            && self.review >= config.max_consecutive_review_failures
        {
            return Some((
                StepPhase::Review,
                self.review,
                config.max_consecutive_review_failures,
            ));
        }
        None
    }
}

/// Daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Path to the `SQLite` database.
    pub db_path: PathBuf,
    /// Maximum concurrent runs (default: 3).
    pub max_concurrent_runs: usize,
    /// Maximum concurrent runs per workspace (optional).
    /// See spec Section 4.2, 5.3: per-workspace cap enforcement.
    pub max_runs_per_workspace: Option<usize>,
    /// HTTP server port (default: 7700).
    pub port: u16,
    /// Auth token for HTTP API (optional, Section 8.1).
    pub auth_token: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            max_concurrent_runs: scheduler::DEFAULT_MAX_CONCURRENT_RUNS,
            max_runs_per_workspace: Some(1),
            port: 7700,
            auth_token: std::env::var("LOOPD_AUTH_TOKEN").ok(),
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
#[derive(Debug)]
pub struct Daemon {
    config: DaemonConfig,
    storage: Arc<Storage>,
    scheduler: Arc<Scheduler>,
    /// Skills metrics per open-skills-orchestration.md Section 7.2.
    skills_metrics: Arc<SkillsMetrics>,
}

impl Daemon {
    /// Create a new daemon with the given configuration.
    pub async fn new(config: DaemonConfig) -> AppResult<Self> {
        let storage = Storage::new(&config.db_path, config.max_concurrent_runs).await?;
        storage.migrate_embedded().await?;
        let storage = Arc::new(storage);

        // Create scheduler with optional per-workspace cap (spec Section 4.2, 5.3).
        let scheduler = Arc::new(match config.max_runs_per_workspace {
            Some(max_per_ws) => Scheduler::new_with_workspace_cap(
                Arc::clone(&storage),
                config.max_concurrent_runs,
                max_per_ws,
            ),
            None => Scheduler::new(Arc::clone(&storage), config.max_concurrent_runs),
        });

        Ok(Self {
            config,
            storage,
            scheduler,
            skills_metrics: Arc::new(SkillsMetrics::new()),
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

    /// Get a reference to the skills metrics.
    pub fn skills_metrics(&self) -> &Arc<SkillsMetrics> {
        &self.skills_metrics
    }

    /// Run the daemon main loop.
    pub async fn run(&self) -> AppResult<()> {
        info!("loopd starting on port {}", self.config.port);
        info!("database: {}", self.config.db_path.display());
        info!("max concurrent runs: {}", self.config.max_concurrent_runs);
        if let Some(limit) = self.config.max_runs_per_workspace {
            info!("max runs per workspace: {}", limit)
        } else {
            info!("max runs per workspace: unbounded")
        }
        if self.config.auth_token.is_some() {
            info!("auth token: enabled");
        }

        // Resume any runs that were interrupted by a previous crash.
        match self.scheduler.resume_interrupted_runs().await {
            Ok(resumed) => {
                if !resumed.is_empty() {
                    info!("resumed {} interrupted run(s)", resumed.len());
                    for run in &resumed {
                        info!("  - {} ({})", run.name, run.id);
                    }
                    // Spawn processing tasks for each resumed run.
                    for run in resumed {
                        let scheduler = Arc::clone(&self.scheduler);
                        let storage = Arc::clone(&self.storage);
                        let skills_metrics = Arc::clone(&self.skills_metrics);
                        let run_id = run.id.clone();
                        let cancel_token = scheduler.cancel_token();
                        tokio::spawn(async move {
                            let scheduler_for_error = Arc::clone(&scheduler);
                            let storage_for_error = Arc::clone(&storage);
                            if let Err(e) =
                                process_run(scheduler, storage, run, cancel_token, skills_metrics)
                                    .await
                            {
                                let error_message = e.to_string();
                                error!("resumed run processing failed: {}", error_message);
                                let run_id = run_id.clone();
                                tokio::spawn(async move {
                                    // On shutdown, leave runs as RUNNING for resume on restart.
                                    if scheduler_for_error.is_shutdown() {
                                        return;
                                    }
                                    if let Ok(current) = storage_for_error.get_run(&run_id).await {
                                        if current.status == loop_core::RunStatus::Canceled {
                                            return;
                                        }
                                    }
                                    let payload = EventPayload::RunFailed(RunFailedPayload {
                                        run_id: run_id.clone(),
                                        reason: format!("run_error:{error_message}"),
                                    });
                                    let _ = storage_for_error
                                        .append_event(&run_id, None, &payload)
                                        .await;
                                    if let Ok(current) = storage_for_error.get_run(&run_id).await {
                                        if current.status != loop_core::RunStatus::Running {
                                            return;
                                        }
                                    }
                                    let _ = scheduler_for_error
                                        .release_run(&run_id, loop_core::RunStatus::Failed)
                                        .await;
                                });
                            }
                        });
                    }
                }
            }
            Err(e) => {
                warn!("failed to resume interrupted runs: {}", e);
            }
        }

        // Start HTTP server in background task.
        let http_storage = Arc::clone(&self.storage);
        let http_scheduler = Arc::clone(&self.scheduler);
        let http_port = self.config.port;
        let http_token = self.config.auth_token.clone();
        let http_handle = tokio::spawn(async move {
            if let Err(e) =
                server::start_server(http_storage, http_scheduler, http_port, http_token).await
            {
                error!("HTTP server error: {}", e);
            }
        });

        // Main scheduling loop.
        loop {
            if self.scheduler.is_shutdown() {
                info!("shutdown signal received, exiting");
                break;
            }

            // Try to claim the next pending run.
            match self.scheduler.claim_next_run().await {
                Ok(Some(run)) => {
                    info!(
                        run_id = %run.id,
                        run_name = %run.name,
                        workspace = %run.workspace_root,
                        "claimed run"
                    );

                    // Spawn a task to process this run.
                    let scheduler = Arc::clone(&self.scheduler);
                    let storage = Arc::clone(&self.storage);
                    let skills_metrics = Arc::clone(&self.skills_metrics);
                    let run_id = run.id.clone();
                    let cancel_token = scheduler.cancel_token();
                    tokio::spawn(async move {
                        let scheduler_for_error = Arc::clone(&scheduler);
                        let storage_for_error = Arc::clone(&storage);
                        if let Err(e) =
                            process_run(scheduler, storage, run, cancel_token, skills_metrics).await
                        {
                            let error_message = e.to_string();
                            error!("run processing failed: {}", error_message);
                            let run_id = run_id.clone();
                            tokio::spawn(async move {
                                // On shutdown, leave runs as RUNNING for resume on restart.
                                if scheduler_for_error.is_shutdown() {
                                    info!(
                                        run_id = %run_id,
                                        "daemon shutting down; leaving run as RUNNING for resume"
                                    );
                                    return;
                                }
                                if let Ok(current) = storage_for_error.get_run(&run_id).await {
                                    if current.status == loop_core::RunStatus::Canceled {
                                        warn!(
                                            run_id = %run_id,
                                            "run canceled; skipping failure transition"
                                        );
                                        return;
                                    }
                                }
                                let payload = EventPayload::RunFailed(RunFailedPayload {
                                    run_id: run_id.clone(),
                                    reason: format!("run_error:{error_message}"),
                                });
                                if let Err(err) = storage_for_error
                                    .append_event(&run_id, None, &payload)
                                    .await
                                {
                                    warn!(
                                        run_id = %run_id,
                                        error = %err,
                                        "failed to record run failure"
                                    );
                                }
                                if let Ok(current) = storage_for_error.get_run(&run_id).await {
                                    if current.status != loop_core::RunStatus::Running {
                                        warn!(
                                            run_id = %run_id,
                                            status = %current.status.as_str(),
                                            "run no longer running; skipping release"
                                        );
                                        return;
                                    }
                                }
                                if let Err(err) = scheduler_for_error
                                    .release_run(&run_id, loop_core::RunStatus::Failed)
                                    .await
                                {
                                    warn!(
                                        run_id = %run_id,
                                        error = %err,
                                        "failed to release failed run"
                                    );
                                }
                            });
                        }
                    });
                }
                Ok(None) => {
                    // No pending runs; sleep before checking again.
                    tokio::time::sleep(CLAIM_POLL_INTERVAL).await;
                }
                Err(scheduler::SchedulerError::Shutdown) => {
                    info!("scheduler shutdown");
                    break;
                }
                Err(e) => {
                    error!("scheduler error: {}", e);
                    tokio::time::sleep(SCHEDULER_ERROR_BACKOFF).await;
                }
            }
        }

        // Grace period for in-flight steps to abort.
        // The cancel token was already signalled in scheduler.shutdown().
        info!(
            grace_period_secs = SHUTDOWN_GRACE_PERIOD.as_secs(),
            "waiting for in-flight steps to abort"
        );
        tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;

        // Abort HTTP server on shutdown.
        http_handle.abort();

        Ok(())
    }

    /// Signal the daemon to shut down.
    pub fn shutdown(&self) {
        info!("shutdown requested");
        self.scheduler.shutdown();
    }
}

/// Get the run directory for artifacts.
/// Follows spec Section 3.2: `<workspace_root>/logs/loop/run-<run_id>/`
fn run_dir(workspace_root: &Path, run_id: &Id) -> PathBuf {
    loop_core::workspace_run_dir(workspace_root, run_id)
}

/// Load run configuration from stored JSON or config files.
fn load_run_config(run: &loop_core::Run) -> AppResult<Config> {
    if let Some(config_json) = run.config_json.as_ref() {
        match serde_json::from_str::<Config>(config_json) {
            Ok(config) => return Ok(config),
            Err(e) => {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "config_json is not valid JSON; falling back to file lookup"
                );
            }
        }

        let config_path = Path::new(config_json);
        if config_path.exists() {
            return Ok(Config::from_file(config_path)?);
        }
    }

    let workspace_root = Path::new(&run.workspace_root);
    let config_path = workspace_root.join(".loop/config");
    if config_path.exists() {
        return Ok(Config::from_file(&config_path)?);
    }

    Ok(Config::default())
}

fn build_worktree_config_for_provider(
    config: &Config,
    workspace_root: &Path,
    run_name: &str,
    spec_path: &Path,
    provider: WorktreeProvider,
) -> AppResult<loop_core::RunWorktree> {
    let mut config_for_worktree = config.clone();
    if provider == WorktreeProvider::Worktrunk {
        if let Some(template) = worktree_worktrunk::resolve_worktree_path_template(config) {
            config_for_worktree.worktree_path_template = template;
        }
    }

    let mut worktree =
        git::build_worktree_config(&config_for_worktree, workspace_root, run_name, spec_path)?;
    worktree.provider = provider;
    Ok(worktree)
}

/// Remap an absolute path from `workspace_root` into the worktree directory.
///
/// When a run uses a worktree, files like the spec and plan exist in both the
/// main working tree and the worktree checkout. The agent must read/write the
/// worktree copy so that changes stay on the run branch instead of leaking as
/// uncommitted modifications in the main working tree.
fn remap_to_worktree(path: &str, workspace_root: &str, worktree_path: &str) -> String {
    if let Some(rel) = path.strip_prefix(workspace_root) {
        let rel = rel.strip_prefix('/').unwrap_or(rel);
        format!("{worktree_path}/{rel}")
    } else {
        path.to_string()
    }
}

/// Build the implementation prompt with context file references.
/// Matches bin/loop behavior: @spec @plan @runner-notes @LEARNINGS.md + `context_files`.
///
/// If skills are provided:
/// - Includes the available_skills XML block per open-skills-orchestration.md Section 4.2 and 5.1
/// - Selects skills for the current task and loads their bodies in OpenSkills `read` format
///
/// Returns (prompt, skill_selection, truncation_events, load_failure_events) where:
/// - skill_selection should be emitted as a SKILLS_SELECTED event per spec Section 4.3
/// - truncation_events should be emitted as SKILLS_TRUNCATED events per spec Section 4.3
/// - load_failure_events should be emitted as SKILLS_LOAD_FAILED events per spec Section 4.3
fn build_implementation_prompt(
    run: &loop_core::Run,
    run_dir: &Path,
    config: &Config,
    available_skills: &[SkillMetadata],
) -> (
    String,
    Option<SkillSelection>,
    Vec<TruncationEvent>,
    Vec<LoadFailureEvent>,
) {
    // When a worktree is active, remap file references so the agent reads/writes
    // the worktree copy instead of the main working tree. Without this, absolute
    // paths like spec_path/plan_path point into workspace_root and any edits the
    // agent makes (e.g. checking off plan items) leak as uncommitted changes on main.
    let remap = |path: &str| -> String {
        if let Some(wt) = run.worktree.as_ref() {
            remap_to_worktree(path, &run.workspace_root, &wt.worktree_path)
        } else {
            path.to_string()
        }
    };

    let spec_ref = remap(&run.spec_path);
    let mut refs = format!("@{spec_ref}");

    if let Some(plan_path) = &run.plan_path {
        refs.push_str(&format!(" @{}", remap(plan_path)));
    }

    // Add runner notes (created by verifier on failure)
    let runner_notes = run_dir.join("runner-notes.txt");
    refs.push_str(&format!(" @{}", runner_notes.display()));

    for context_path in &config.context_files {
        let remapped = remap(&context_path.display().to_string());
        refs.push_str(&format!(" @{remapped}"));
    }

    // Add learnings file if it exists
    let workspace_root = PathBuf::from(&run.workspace_root);
    let learnings_path = workspace_root.join(&config.specs_dir).join("LEARNINGS.md");
    if learnings_path.exists() {
        refs.push_str(&format!(" @{}", remap(&learnings_path.display().to_string())));
    }

    let custom_prompt = if let Some(prompt_file) = config.prompt_file.as_ref() {
        prompt_file.exists().then(|| prompt_file.clone())
    } else {
        let default_prompt = workspace_root.join(".loop/prompt.txt");
        default_prompt.exists().then_some(default_prompt)
    };

    let completion_note = match config.completion_mode {
        loop_core::CompletionMode::Exact => {
            "The runner detects completion only if your entire output is exactly <promise>COMPLETE</promise>."
        }
        loop_core::CompletionMode::Trailing => {
            "The runner detects completion when the last non-empty line is exactly <promise>COMPLETE</promise>."
        }
    };

    let mut prompt = if let Some(custom_prompt) = custom_prompt {
        match std::fs::read_to_string(&custom_prompt) {
            Ok(content) => content,
            Err(err) => {
                warn!(
                    run_id = %run.id,
                    path = %custom_prompt.display(),
                    error = %err,
                    "failed to read custom prompt, falling back to default"
                );
                String::new()
            }
        }
    } else {
        String::new()
    };

    if prompt.trim().is_empty() {
        prompt = format!(
            r#"{refs}

You are an implementation agent. Read the spec and the plan.

IMPORTANT: Before starting work, check:
1. The LEARNINGS.md file for repo-wide patterns and common mistakes
2. The ## Learnings section at the bottom of the plan for task-specific corrections
Avoid repeating past mistakes - these learnings exist because previous implementations got it wrong.

Task:
1. Choose ONE unchecked task from the plan with the highest priority (not necessarily first).
2. Implement only that task (single feature). Avoid unrelated changes.
3. Run verification relevant to that task. If the plan lists a verification checklist, run what
   applies. If you cannot run a step, add a note to the plan's `## Notes` or `## Blockers Discovered` section.
4. Update the plan checklist: mark only the task(s) you completed with [x]. Leave others untouched.
   Verification checklist items are not tasks: leave them `[ ]` or mark `[R]` when run. Never mark them `[x]`.
5. Make exactly one git commit for your changes using `gritty commit --accept`.
6. If a task is blocked by a production bug or missing test infrastructure, mark it `[~]` and add it to
   the plan's `## Blockers Discovered` section. Do not mark it `[x]`.
7. If (and only if) all `[ ]` and `[~]` tasks in the plan are complete (ignore verification checklists and `[ ]?` manual QA items), respond with:
<promise>COMPLETE</promise>

Checkbox legend:
- `[ ]`: pending (blocks completion)
- `[~]`: blocked (blocks completion)
- `[x]`: implemented, awaiting review
- `[R]`: reviewed/verified (non-blocking)
- `[ ]?`: manual QA only (ignored)

Spec alignment guardrails (must follow):
- Before coding, identify the exact spec section(s) you are implementing and list the required
  behavior, constraints, and any data shapes.
- If the spec defines a schema/event payload/API contract, match it exactly (field names,
  nesting, nullability, ordering). Keep types in sync.
- Do not use placeholder values for required behavior. Implement the real behavior or leave the
  task unchecked.
- If any spec detail is ambiguous, do not guess. Choose the safest minimal interpretation,
  document the assumption in your response, and limit changes to what is unambiguous.

Response format (strict):
- ALL `[ ]` tasks complete: output `<promise>COMPLETE</promise>`.
  If the runner requires exact output, print only the token; otherwise ensure it's the final non-empty line.
- Tasks remain: ONE sentence only.
  - If you completed a task: "Completed [task]. [N] tasks remain."
  - If you marked a task `[~]`: "Blocked [task]. [N] tasks remain."
  (N = unchecked `[ ]` + `[~]` items only)
  Multi-sentence output wastes context and delays completion.

Constraints:
- Do not modify files under `reference/`.
- Do not work on more than one plan item.
- If no changes were made, do not commit.

{completion_note}"#
        );
    }

    let plan_placeholder = run.plan_path.as_deref().map(|p| remap(p)).unwrap_or_default();
    prompt = prompt
        .replace("SPEC_PATH", &remap(&run.spec_path))
        .replace("PLAN_PATH", &plan_placeholder);

    // Select task from plan first (needed for both prompt injection and skill selection).
    // Per open-skills-orchestration.md Section 5.1: parse plan and select task.
    let mut selected_task: Option<TaskSelection> = None;
    if let Some(plan_path) = &run.plan_path {
        let plan_file = PathBuf::from(remap(plan_path));
        if let Ok(Some(task)) = select_task(&plan_file) {
            selected_task = Some(task);
        }
    }

    // Inject selected task text into prompt (Section 5.1).
    // Adds a "Selected Task:" section to provide explicit context about which task to implement.
    if let Some(ref task) = selected_task {
        let section_info = task
            .section
            .as_ref()
            .map(|s| format!("\n(Section: {})", s))
            .unwrap_or_default();
        let task_section = format!(
            "\n\n## Selected Task\n\nWork on this specific task:\n> {}{}\n",
            task.label, section_info
        );
        prompt.push_str(&task_section);
    }

    // Append available skills XML block if skills are provided.
    // Per open-skills-orchestration.md Section 4.2 and 5.1.
    let skills_block = render_available_skills(available_skills);
    if !skills_block.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&skills_block);
    }

    // Select and load skills for the current task.
    // Per open-skills-orchestration.md Section 5.1.
    let mut truncation_events: Vec<TruncationEvent> = Vec::new();
    let mut skill_selection: Option<SkillSelection> = None;
    let mut load_failure_events: Vec<LoadFailureEvent> = Vec::new();

    if !available_skills.is_empty() {
        if let Some(ref task) = selected_task {
            // Parse run ID as UUID for skill selection (fallback to nil if invalid).
            let run_uuid = Uuid::parse_str(&run.id.0).unwrap_or(Uuid::nil());

            // Select skills for this task.
            let selection = select_skills(
                run_uuid,
                task,
                available_skills,
                StepKind::Implementation,
                config.skills_max_selected_impl,
            );

            // Load selected skill bodies in OpenSkills `read` format.
            for selected in &selection.skills {
                if let Some(skill) = available_skills.iter().find(|s| s.name == selected.name) {
                    match load_skill_body(
                        skill,
                        config.skills_load_references,
                        config.skills_max_body_chars,
                    ) {
                        Ok(loaded) => {
                            prompt.push_str("\n\n");
                            prompt.push_str(&loaded.content);
                            if loaded.truncated {
                                warn!(
                                    run_id = %run.id,
                                    skill = %skill.name,
                                    original_size = loaded.original_size.unwrap_or(0),
                                    max_chars = config.skills_max_body_chars,
                                    "skill body truncated"
                                );
                                // Track for SKILLS_TRUNCATED event emission (Section 4.3).
                                truncation_events.push(TruncationEvent {
                                    name: skill.name.clone(),
                                    max_chars: config.skills_max_body_chars,
                                });
                            }
                        }
                        Err(e) => {
                            warn!(
                                run_id = %run.id,
                                skill = %skill.name,
                                path = %skill.path.display(),
                                error = %e,
                                "failed to load skill body"
                            );
                            load_failure_events.push(LoadFailureEvent {
                                name: skill.name.clone(),
                                error: e.to_string(),
                            });
                        }
                    }
                }
            }

            skill_selection = Some(selection);
        }
    }

    (
        prompt,
        skill_selection,
        truncation_events,
        load_failure_events,
    )
}

/// Build the review prompt.
/// Matches bin/loop's `load_reviewer_prompt` behavior.
///
/// If skills are provided:
/// - Includes the available_skills XML block per open-skills-orchestration.md Section 4.2 and 5.1
/// - Selects skills for the current task and loads their bodies in OpenSkills `read` format
///
/// Returns (prompt, skill_selection, truncation_events, load_failure_events) where:
/// - skill_selection should be emitted as a SKILLS_SELECTED event per spec Section 4.3
/// - truncation_events should be emitted as SKILLS_TRUNCATED events per spec Section 4.3
/// - load_failure_events should be emitted as SKILLS_LOAD_FAILED events per spec Section 4.3
fn build_review_prompt(
    run: &loop_core::Run,
    config: &Config,
    available_skills: &[SkillMetadata],
) -> (
    String,
    Option<SkillSelection>,
    Vec<TruncationEvent>,
    Vec<LoadFailureEvent>,
) {
    let remap = |path: &str| -> String {
        if let Some(wt) = run.worktree.as_ref() {
            remap_to_worktree(path, &run.workspace_root, &wt.worktree_path)
        } else {
            path.to_string()
        }
    };

    let spec_ref = remap(&run.spec_path);
    let mut refs = format!("@{spec_ref}");
    if let Some(plan_path) = &run.plan_path {
        refs.push_str(&format!(" @{}", remap(plan_path)));
    }

    let mut prompt = format!(
        r"{refs}

You are a senior staff engineer reviewing implementation work.

## Your Role

Review the recent changes (since the last verified commit) for:
1. Correctness: Does the code do what the spec/plan requires?
2. Quality: Is the code clean, well-structured, and maintainable?
3. Safety: Are there any security issues, bugs, or regressions?

## Response Format

If the changes are acceptable:
- Output exactly: APPROVED

If changes are needed:
- List specific issues that must be fixed
- Be concise but clear about what needs to change
- Do not approve until issues are resolved"
    );

    // Select task from plan first (needed for both prompt injection and skill selection).
    // Per open-skills-orchestration.md Section 5.1: parse plan and select task.
    let mut selected_task: Option<TaskSelection> = None;
    if let Some(plan_path) = &run.plan_path {
        let plan_file = PathBuf::from(remap(plan_path));
        if let Ok(Some(task)) = select_task(&plan_file) {
            selected_task = Some(task);
        }
    }

    // Inject selected task text into prompt (Section 5.1).
    // Provides context about which task is being reviewed.
    if let Some(ref task) = selected_task {
        let section_info = task
            .section
            .as_ref()
            .map(|s| format!("\n(Section: {})", s))
            .unwrap_or_default();
        let task_section = format!(
            "\n\n## Task Under Review\n\nThe implementation is for this task:\n> {}{}\n",
            task.label, section_info
        );
        prompt.push_str(&task_section);
    }

    // Append available skills XML block if skills are provided.
    // Per open-skills-orchestration.md Section 4.2 and 5.1.
    let skills_block = render_available_skills(available_skills);
    if !skills_block.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&skills_block);
    }

    // Select and load skills for the current task.
    // Per open-skills-orchestration.md Section 5.1.
    let mut truncation_events: Vec<TruncationEvent> = Vec::new();
    let mut skill_selection: Option<SkillSelection> = None;
    let mut load_failure_events: Vec<LoadFailureEvent> = Vec::new();

    if !available_skills.is_empty() {
        if let Some(ref task) = selected_task {
            // Parse run ID as UUID for skill selection (fallback to nil if invalid).
            let run_uuid = Uuid::parse_str(&run.id.0).unwrap_or(Uuid::nil());

            // Select skills for this task with review limit.
            let selection = select_skills(
                run_uuid,
                task,
                available_skills,
                StepKind::Review,
                config.skills_max_selected_review,
            );

            // Load selected skill bodies in OpenSkills `read` format.
            for selected in &selection.skills {
                if let Some(skill) = available_skills.iter().find(|s| s.name == selected.name) {
                    match load_skill_body(
                        skill,
                        config.skills_load_references,
                        config.skills_max_body_chars,
                    ) {
                        Ok(loaded) => {
                            prompt.push_str("\n\n");
                            prompt.push_str(&loaded.content);
                            if loaded.truncated {
                                warn!(
                                    run_id = %run.id,
                                    skill = %skill.name,
                                    original_size = loaded.original_size.unwrap_or(0),
                                    max_chars = config.skills_max_body_chars,
                                    "skill body truncated"
                                );
                                // Track for SKILLS_TRUNCATED event emission (Section 4.3).
                                truncation_events.push(TruncationEvent {
                                    name: skill.name.clone(),
                                    max_chars: config.skills_max_body_chars,
                                });
                            }
                        }
                        Err(e) => {
                            warn!(
                                run_id = %run.id,
                                skill = %skill.name,
                                path = %skill.path.display(),
                                error = %e,
                                "failed to load skill body"
                            );
                            load_failure_events.push(LoadFailureEvent {
                                name: skill.name.clone(),
                                error: e.to_string(),
                            });
                        }
                    }
                }
            }

            skill_selection = Some(selection);
        }
    }

    (
        prompt,
        skill_selection,
        truncation_events,
        load_failure_events,
    )
}

/// Write summary.json for a run if enabled in config.
///
/// Implements postmortem-analysis.md Section 5.1 step 2.
async fn maybe_export_report(storage: &Storage, run: &Run, config: &Config) {
    let workspace_root = Path::new(&run.workspace_root);
    let run_dir = loop_core::workspace_run_dir(workspace_root, &run.id);
    let report_path = run_dir.join("report.tsv");

    if let Err(e) = std::fs::create_dir_all(&run_dir) {
        warn!(
            run_id = %run.id,
            error = %e,
            "failed to create run directory for report export"
        );
        return;
    }

    if let Err(e) = storage.export_report(&run.id, &report_path).await {
        warn!(
            run_id = %run.id,
            error = %e,
            "failed to export report.tsv"
        );
        return;
    }

    match mirror_artifact(
        &run.id,
        "report",
        &report_path,
        &config.global_log_dir,
        config.artifact_mode,
    ) {
        Ok(artifacts) => {
            for artifact in artifacts {
                if let Err(e) = storage.insert_artifact(&artifact).await {
                    warn!(
                        run_id = %run.id,
                        error = %e,
                        "failed to register report.tsv artifact"
                    );
                }
            }
            info!(run_id = %run.id, "report.tsv written");
        }
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to mirror report.tsv"
            );
        }
    }
}

async fn maybe_write_summary(
    storage: &Storage,
    run: &Run,
    config: &Config,
    exit_reason: ExitReason,
    last_exit_code: i32,
    completion_mode: Option<&str>,
) {
    if !config.summary_json {
        return;
    }

    match postmortem::write_summary_json(
        storage,
        run,
        config,
        exit_reason,
        last_exit_code,
        completion_mode,
    )
    .await
    {
        Ok(artifacts) => {
            for artifact in artifacts {
                if let Err(e) = storage.insert_artifact(&artifact).await {
                    warn!(
                        run_id = %run.id,
                        error = %e,
                        "failed to register summary.json artifact"
                    );
                }
            }
            info!(run_id = %run.id, "summary.json written");
        }
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to write summary.json"
            );
        }
    }
}

async fn maybe_write_review_snapshot(
    storage: &Storage,
    run: &Run,
    config: &Config,
    exit_reason: ExitReason,
) {
    if !matches!(exit_reason, ExitReason::CompletePlan | ExitReason::CompleteReviewer) {
        return;
    }

    let Some(worktree) = run.worktree.as_ref() else {
        return;
    };

    let workspace_root = Path::new(&run.workspace_root);
    let snapshot = match build_run_diff_snapshot(workspace_root, worktree) {
        Ok(snapshot) => snapshot,
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to build review diff snapshot"
            );
            return;
        }
    };

    let json = match serde_json::to_vec(&snapshot) {
        Ok(json) => json,
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to serialize review diff snapshot"
            );
            return;
        }
    };

    if json.len() > MAX_REVIEW_SNAPSHOT_BYTES {
        warn!(
            run_id = %run.id,
            bytes = json.len(),
            limit = MAX_REVIEW_SNAPSHOT_BYTES,
            "review diff snapshot exceeds size limit"
        );
        return;
    }

    match write_and_mirror_artifact(
        &run.id,
        "review_diff",
        "review-diff.json",
        &json,
        workspace_root,
        &config.global_log_dir,
        config.artifact_mode,
    ) {
        Ok(artifacts) => {
            for artifact in artifacts {
                if let Err(e) = storage.insert_artifact(&artifact).await {
                    warn!(
                        run_id = %run.id,
                        error = %e,
                        "failed to register review diff snapshot artifact"
                    );
                }
            }
            info!(run_id = %run.id, "review diff snapshot written");
        }
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to write review diff snapshot"
            );
        }
    }
}

async fn finalize_run_artifacts(
    storage: &Storage,
    run: &Run,
    config: &Config,
    exit_reason: ExitReason,
    last_exit_code: i32,
    completion_mode: Option<&str>,
) {
    maybe_export_report(storage, run, config).await;
    maybe_write_summary(
        storage,
        run,
        config,
        exit_reason,
        last_exit_code,
        completion_mode,
    )
    .await;
    maybe_write_review_snapshot(storage, run, config, exit_reason).await;
}

/// Run postmortem analysis if enabled in config.
///
/// Implements postmortem-analysis.md Section 5.1 steps 3-5.
/// Emits `POSTMORTEM_START` and `POSTMORTEM_END` events.
async fn maybe_run_postmortem(
    storage: &Storage,
    run: &Run,
    config: &Config,
    iterations_run: u32,
    completed_iter: Option<u32>,
    reason: &str,
) {
    if !config.postmortem {
        return;
    }

    // Check if claude is available (spec Section 5.1 step 3)
    if !postmortem::is_claude_available() {
        warn!(
            run_id = %run.id,
            "claude CLI not found; skipping postmortem analysis"
        );
        return;
    }

    // Emit POSTMORTEM_START event
    let start_event = EventPayload::PostmortemStart(PostmortemStartPayload {
        run_id: run.id.clone(),
        reason: reason.to_string(),
    });
    if let Err(e) = storage.append_event(&run.id, None, &start_event).await {
        warn!(
            run_id = %run.id,
            error = %e,
            "failed to emit POSTMORTEM_START event"
        );
    }

    // Run the postmortem analysis pipeline
    let status =
        match postmortem::run_postmortem_analysis(run, config, iterations_run, completed_iter) {
            Ok(result) => {
                if result.all_succeeded() {
                    info!(run_id = %run.id, "postmortem analysis completed successfully");
                    "ok"
                } else {
                    warn!(run_id = %run.id, "postmortem analysis partially failed");
                    "failed"
                }
            }
            Err(e) => {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "postmortem analysis failed"
                );
                "failed"
            }
        };

    // Emit POSTMORTEM_END event
    let end_event = EventPayload::PostmortemEnd(PostmortemEndPayload {
        run_id: run.id.clone(),
        status: status.to_string(),
    });
    if let Err(e) = storage.append_event(&run.id, None, &end_event).await {
        warn!(
            run_id = %run.id,
            error = %e,
            "failed to emit POSTMORTEM_END event"
        );
    }
}

/// Process a single run through all phases.
///
/// Implements the main flow from spec Section 5.1:
/// implementation -> review -> verification -> (watchdog if signals) -> completion
///
/// Registers a per-run cancellation token that fires when either the run is
/// individually cancelled or the global shutdown token fires.
async fn process_run(
    scheduler: Arc<Scheduler>,
    storage: Arc<Storage>,
    run: loop_core::Run,
    _cancel_token: tokio_util::sync::CancellationToken,
    skills_metrics: Arc<SkillsMetrics>,
) -> AppResult<()> {
    // Register a per-run cancellation token (child of global shutdown token).
    // This allows cancel_run() to kill just this run's in-flight process.
    let cancel_token = scheduler.register_run_token(&run.id).await;

    let result = process_run_inner(
        Arc::clone(&scheduler),
        storage,
        run.clone(),
        cancel_token,
        skills_metrics,
    )
    .await;

    // Always clean up the per-run token on exit.
    scheduler.remove_run_token(&run.id).await;

    result
}

/// Inner implementation of process_run, separated so the per-run token
/// cleanup in the outer function runs on all exit paths.
async fn process_run_inner(
    scheduler: Arc<Scheduler>,
    storage: Arc<Storage>,
    run: loop_core::Run,
    cancel_token: tokio_util::sync::CancellationToken,
    skills_metrics: Arc<SkillsMetrics>,
) -> AppResult<()> {
    info!(
        run_id = %run.id,
        run_name = %run.name,
        workspace = %run.workspace_root,
        spec = %run.spec_path,
        "processing run"
    );

    let mut run = run;
    let workspace_root_path = std::path::Path::new(&run.workspace_root);

    // Parse run config.
    let mut config = load_run_config(&run)?;
    config.resolve_paths(workspace_root_path);

    // Initialize consecutive failure counters from step history (consecutive-failure-detection.md Section 3.3).
    // This handles daemon restarts by rebuilding state from persisted steps.
    let steps = storage.list_steps(&run.id).await?;
    let mut consecutive_failures = ConsecutiveFailures::from_steps(&steps);
    info!(
        run_id = %run.id,
        verification = consecutive_failures.verification,
        review = consecutive_failures.review,
        "initialized consecutive failure counters"
    );

    // Sync built-in skills if enabled (open-skills-orchestration.md Section 5.1).
    if config.skills_enabled && config.skills_sync_on_start {
        if let Err(e) =
            skills::sync_builtin_skills(&config.skills_builtin_dir, &config.skills_sync_dir)
        {
            // Per spec Section 5.2: log and continue on sync failure.
            warn!(
                run_id = %run.id,
                src = %config.skills_builtin_dir.display(),
                dst = %config.skills_sync_dir.display(),
                error = %e,
                "built-in skill sync failed, continuing with repo directory"
            );
        }
    }

    // Discover available skills if enabled (open-skills-orchestration.md Section 5.1).
    let discovered_skills: Vec<SkillMetadata> = if config.skills_enabled {
        let discovery = skills::discover_skills(&config, workspace_root_path);

        // Increment metrics per Section 7.2.
        skills_metrics.inc_discovered(discovery.skills.len());

        info!(
            run_id = %run.id,
            count = discovery.skills.len(),
            errors = discovery.errors.len(),
            "discovered skills"
        );
        for error in &discovery.errors {
            warn!(
                run_id = %run.id,
                skill = %error.name,
                path = %error.path.display(),
                error = %error.error,
                "skill discovery error"
            );
            skills_metrics.inc_load_failed();
            let payload = EventPayload::SkillsLoadFailed(SkillsLoadFailedPayload {
                run_id: run.id.clone(),
                name: error.name.clone(),
                error: error.error.to_string(),
            });
            if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "failed to emit SKILLS_LOAD_FAILED event"
                );
            }
        }

        // Emit SKILLS_DISCOVERED event (Section 4.3).
        if !discovery.skills.is_empty() {
            let locations: Vec<String> = discovery
                .skills
                .iter()
                .map(|s| s.location.as_str().to_string())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            let names: Vec<String> = discovery.skills.iter().map(|s| s.name.clone()).collect();
            let discovered_event = EventPayload::SkillsDiscovered(SkillsDiscoveredPayload {
                run_id: run.id.clone(),
                count: discovery.skills.len(),
                locations,
                names,
            });
            if let Err(e) = storage.append_event(&run.id, None, &discovered_event).await {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "failed to emit SKILLS_DISCOVERED event"
                );
            }
        }

        discovery.skills
    } else {
        Vec::new()
    };

    // Resolve worktree provider and emit event (worktrunk-integration.md Section 5.2).
    let resolved_provider = worktree::resolve_provider(&config, workspace_root_path)?;
    info!(
        run_id = %run.id,
        provider = ?resolved_provider,
        "worktree provider resolved"
    );

    // Emit WORKTREE_PROVIDER_SELECTED event (Section 4.3).
    let provider_event = EventPayload::WorktreeProviderSelected(WorktreeProviderSelectedPayload {
        run_id: run.id.clone(),
        provider: resolved_provider,
    });
    storage.append_event(&run.id, None, &provider_event).await?;

    let needs_worktree_update = match run.worktree.as_ref() {
        Some(worktree) => worktree.provider != resolved_provider,
        None => true,
    };

    if needs_worktree_update {
        let worktree = build_worktree_config_for_provider(
            &config,
            workspace_root_path,
            &run.name,
            Path::new(&run.spec_path),
            resolved_provider,
        )?;
        storage.update_run_worktree(&run.id, &worktree).await?;
        run.worktree = Some(worktree);
    }

    // Set up paths.
    let workspace_root = PathBuf::from(&run.workspace_root);
    let run_dir = run_dir(&workspace_root, &run.id);
    std::fs::create_dir_all(&run_dir)?;

    // Create worktree if configured (worktrunk-integration.md Section 5.1).
    // Uses the resolved provider to create the worktree before execution.
    if let Some(ref worktree_config) = run.worktree {
        // Pre-flight check: reject if working tree has uncommitted changes.
        // The worktree branches from HEAD, so uncommitted changes would be excluded
        // from the run, which is confusing. Fail fast to avoid this.
        if !git::is_working_tree_clean(&workspace_root)? {
            eyre::bail!(
                "cannot create worktree: working tree has uncommitted changes. \
                 Commit or stash your changes before starting a run."
            );
        }

        info!(
            run_id = %run.id,
            provider = ?resolved_provider,
            worktree_path = %worktree_config.worktree_path,
            run_branch = %worktree_config.run_branch,
            "creating worktree"
        );

        // Create a RunWorktree with the resolved provider for the prepare call.
        let mut worktree_with_provider = worktree_config.clone();
        worktree_with_provider.provider = resolved_provider;

        worktree::prepare(&workspace_root, &worktree_with_provider, &config)?;

        // Emit WORKTREE_CREATED event (Section 4.3).
        let created_event = EventPayload::WorktreeCreated(WorktreeCreatedPayload {
            run_id: run.id.clone(),
            provider: resolved_provider,
            worktree_path: worktree_config.worktree_path.clone(),
            run_branch: worktree_config.run_branch.clone(),
        });
        storage.append_event(&run.id, None, &created_event).await?;

        info!(
            run_id = %run.id,
            worktree_path = %worktree_config.worktree_path,
            "worktree created"
        );
    }

    // Determine working directory (worktree if configured, else workspace root).
    let working_dir = run.worktree.as_ref().map_or_else(
        || workspace_root.clone(),
        |wt| PathBuf::from(&wt.worktree_path),
    );

    // Belt-and-suspenders: verify working_dir is actually on the run branch.
    // Catches any mismatch even if prepare() validation passed (e.g. race condition,
    // external modification, or a provider bug).
    if let Some(ref wt) = run.worktree {
        git::verify_worktree_branch(&working_dir, &wt.run_branch).map_err(|e| {
            eyre::eyre!(
                "working_dir {} is not on expected branch {}: {e}",
                working_dir.display(),
                wt.run_branch
            )
        })?;
    }

    // Create runners and verifier from config.
    // Implementation and review may use different models (review_model config key).
    let runner = Runner::new(RunnerConfig::from_config(&config));
    let review_runner = Runner::new(RunnerConfig::from_config_for_review(&config));
    let verifier = Verifier::new(VerifierConfig::from_config(&config));
    let watchdog = Watchdog::with_defaults();

    let mut previous_outputs: Vec<String> = Vec::new();
    let mut last_output: Option<String> = None;
    let mut last_prompt: Option<String> = None;
    let mut pending_rewrite: Option<watchdog::RewriteResult> = None;
    let mut rewrite_count: u32 = 0;

    // Ensure runner-notes.txt exists (empty initially).
    Verifier::clear_runner_notes(&run_dir)?;

    // Track iteration count for iteration limit.
    let mut iteration_count = 0u32;

    // Track last exit code for summary.json (postmortem-analysis.md Section 3).
    let mut last_exit_code: i32 = 0;

    // Main phase loop.
    loop {
        // Check iteration limit.
        if iteration_count >= config.iterations {
            warn!(
                run_id = %run.id,
                iterations = iteration_count,
                limit = config.iterations,
                "iteration limit reached"
            );
            // Write report + summary.json before emitting events (postmortem-analysis.md Section 5.1).
            finalize_run_artifacts(
                &storage,
                &run,
                &config,
                ExitReason::IterationsExhausted,
                last_exit_code,
                Some(config.completion_mode.as_str()),
            )
            .await;
            // Emit RUN_FAILED event and update status atomically (Section 4.3).
            // Complete run BEFORE postmortem to free capacity immediately.
            let event_payload = EventPayload::RunFailed(RunFailedPayload {
                run_id: run.id.clone(),
                reason: format!("iteration_limit_reached:{}", config.iterations),
            });
            scheduler
                .complete_run(&run.id, loop_core::RunStatus::Failed, &event_payload)
                .await?;
            // Run postmortem analysis (postmortem-analysis.md Section 5.1).
            maybe_run_postmortem(
                &storage,
                &run,
                &config,
                iteration_count,
                None,
                "iterations_exhausted",
            )
            .await;
            break;
        }

        // Determine the next phase.
        let next_phase = scheduler.determine_next_phase(&run.id).await?;

        let Some(phase) = next_phase else {
            // No more phases; run is complete (merge was terminal).
            info!("run complete: {}", run.id);
            // Write report + summary.json before emitting events (postmortem-analysis.md Section 5.1).
            finalize_run_artifacts(
                &storage,
                &run,
                &config,
                ExitReason::CompletePlan,
                last_exit_code,
                Some(config.completion_mode.as_str()),
            )
            .await;
            // Emit RUN_COMPLETED event and update status atomically (Section 4.3).
            // Complete run BEFORE postmortem to free capacity immediately.
            let event_payload = EventPayload::RunCompleted(RunCompletedPayload {
                run_id: run.id.clone(),
                mode: "merge".to_string(),
            });
            scheduler
                .complete_run(&run.id, loop_core::RunStatus::Completed, &event_payload)
                .await?;
            // Run postmortem analysis (postmortem-analysis.md Section 5.1).
            maybe_run_postmortem(
                &storage,
                &run,
                &config,
                iteration_count,
                Some(iteration_count),
                "run_completed",
            )
            .await;
            break;
        };

        // Enqueue and execute the step.
        let current_run = storage.get_run(&run.id).await?;
        if current_run.status == loop_core::RunStatus::Canceled {
            info!(run_id = %run.id, "run canceled; stopping execution");
            finalize_run_artifacts(
                &storage,
                &run,
                &config,
                ExitReason::Canceled,
                last_exit_code,
                Some(config.completion_mode.as_str()),
            )
            .await;
            maybe_run_postmortem(
                &storage,
                &run,
                &config,
                iteration_count,
                None,
                "run_canceled",
            )
            .await;
            break;
        }
        if current_run.status != loop_core::RunStatus::Running {
            warn!(
                run_id = %run.id,
                status = %current_run.status.as_str(),
                "run no longer running; stopping execution"
            );
            break;
        }

        let step = scheduler.enqueue_step(&run.id, phase).await?;
        info!(
            run_name = %run.name,
            step_id = %step.id,
            phase = ?step.phase,
            attempt = step.attempt,
            "starting step"
        );

        scheduler.start_step(&step.id).await?;

        // Emit STEP_STARTED event for all phases (Section 4.3).
        let step_started_payload = EventPayload::StepStarted(StepStartedPayload {
            step_id: step.id.clone(),
            phase: phase.as_str().to_string(),
            attempt: step.attempt,
        });
        storage
            .append_event(&run.id, Some(&step.id), &step_started_payload)
            .await?;

        match phase {
            StepPhase::Implementation => {
                iteration_count += 1;

                // Build and write prompt.
                let (prompt, prompt_path) = if let Some(rewrite) = pending_rewrite.take() {
                    (rewrite.content.clone(), rewrite.prompt_after.clone())
                } else {
                    let (prompt, skill_selection, truncation_events, load_failure_events) =
                        build_implementation_prompt(&run, &run_dir, &config, &discovered_skills);

                    // Emit SKILLS_LOAD_FAILED events and increment metrics per Section 4.3 / 7.2.
                    for failure in &load_failure_events {
                        skills_metrics.inc_load_failed();
                        let payload = EventPayload::SkillsLoadFailed(SkillsLoadFailedPayload {
                            run_id: run.id.clone(),
                            name: failure.name.clone(),
                            error: failure.error.clone(),
                        });
                        if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                            warn!(
                                run_id = %run.id,
                                error = %e,
                                "failed to emit SKILLS_LOAD_FAILED event"
                            );
                        }
                    }

                    // Emit SKILLS_SELECTED event (Section 4.3) and log selection decisions (Section 7.1).
                    if let Some(ref selection) = skill_selection {
                        // Log selection decisions per Section 7.1.
                        if !selection.skills.is_empty() {
                            info!(
                                run_id = %run.id,
                                step_kind = %selection.step_kind.as_str(),
                                task = %selection.task_label,
                                count = selection.skills.len(),
                                "selected skills for task"
                            );
                            for skill in &selection.skills {
                                info!(
                                    run_id = %run.id,
                                    skill = %skill.name,
                                    reason = %skill.reason,
                                    "skill selected"
                                );
                            }
                        }

                        if !selection.errors.is_empty() {
                            warn!(
                                run_id = %run.id,
                                step_kind = %selection.step_kind.as_str(),
                                task = %selection.task_label,
                                errors = ?selection.errors,
                                "skill selection errors"
                            );
                        }

                        // Increment metrics per Section 7.2.
                        skills_metrics.inc_selected(selection.skills.len());

                        let payload = EventPayload::SkillsSelected(SkillsSelectedPayload {
                            run_id: run.id.clone(),
                            step_kind: selection.step_kind.as_str().to_string(),
                            task_label: selection.task_label.clone(),
                            skills: selection
                                .skills
                                .iter()
                                .map(|s| SelectedSkillPayload {
                                    name: s.name.clone(),
                                    reason: s.reason.clone(),
                                })
                                .collect(),
                            strategy: match selection.strategy {
                                skills::SelectionStrategy::Hint => "hint".to_string(),
                                skills::SelectionStrategy::Match => "match".to_string(),
                                skills::SelectionStrategy::None => "none".to_string(),
                            },
                            errors: selection.errors.clone(),
                        });
                        if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                            warn!(
                                run_id = %run.id,
                                error = %e,
                                "failed to emit SKILLS_SELECTED event"
                            );
                        }
                    }

                    // Emit SKILLS_TRUNCATED events (Section 4.3) and increment metrics (Section 7.2).
                    for event in &truncation_events {
                        skills_metrics.inc_truncated();
                        let payload = EventPayload::SkillsTruncated(SkillsTruncatedPayload {
                            run_id: run.id.clone(),
                            name: event.name.clone(),
                            max_chars: event.max_chars,
                        });
                        if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                            warn!(
                                run_id = %run.id,
                                error = %e,
                                "failed to emit SKILLS_TRUNCATED event"
                            );
                        }
                    }

                    let artifacts = write_and_mirror_artifact(
                        &run.id,
                        "prompt",
                        "prompt.txt",
                        prompt.as_bytes(),
                        &workspace_root,
                        &config.global_log_dir,
                        config.artifact_mode,
                    )?;
                    insert_artifacts(&storage, artifacts).await?;
                    (prompt, run_dir.join("prompt.txt"))
                };

                last_prompt = Some(prompt.clone());

                info!(
                    step_id = %step.id,
                    prompt_path = %prompt_path.display(),
                    "wrote implementation prompt"
                );

                // Capture HEAD before step for diff stats.
                let head_before = git::get_head_commit(&working_dir).ok();

                // Execute via runner.
                match runner
                    .execute_step(&step, &prompt, &run_dir, &working_dir, cancel_token.clone())
                    .await
                {
                    Ok(result) => {
                        // Track last exit code for summary.json.
                        last_exit_code = result.exit_code;

                        // Log diff stats for this iteration.
                        if let Some(ref before) = head_before {
                            if let Ok(stats) = git::diff_stats_between(&working_dir, before, "HEAD")
                            {
                                info!(
                                    step_id = %step.id,
                                    phase = "implementation",
                                    stats = %stats,
                                    "step complete"
                                );
                            }
                        }

                        // Record step completion.
                        scheduler
                            .complete_step(
                                &step.id,
                                StepStatus::Succeeded,
                                Some(result.exit_code),
                                Some(result.output_path.to_string_lossy().as_ref()),
                            )
                            .await?;

                        // Emit STEP_FINISHED event.
                        let event_payload = EventPayload::StepFinished(StepFinishedPayload {
                            step_id: step.id.clone(),
                            exit_code: result.exit_code,
                            duration_ms: result.duration_ms,
                            output_path: result.output_path.to_string_lossy().to_string(),
                        });
                        storage
                            .append_event(&run.id, Some(&step.id), &event_payload)
                            .await?;

                        // Persist output artifacts (mirror if configured).
                        let output_artifacts = mirror_artifact(
                            &run.id,
                            "implementation_output",
                            &result.output_path,
                            &config.global_log_dir,
                            config.artifact_mode,
                        )?;
                        insert_artifacts(&storage, output_artifacts).await?;

                        let tail_artifacts = mirror_artifact(
                            &run.id,
                            "implementation_tail",
                            &result.tail_path,
                            &config.global_log_dir,
                            config.artifact_mode,
                        )?;
                        insert_artifacts(&storage, tail_artifacts).await?;

                        // Track output for watchdog evaluation after verification.
                        last_output = Some(result.output.clone());

                        // Check for completion token.
                        let completion_result =
                            check_completion(&result.output, config.completion_mode);
                        if completion_result.is_complete {
                            info!(
                                run_id = %run.id,
                                "completion token detected"
                            );

                            // Check if merge is configured (Section 5.3).
                            // If merge_target_branch is set and strategy is not None,
                            // we need to execute the merge phase before completing.
                            let needs_merge = run.worktree.as_ref().is_some_and(|wt| {
                                wt.merge_target_branch.is_some()
                                    && wt.merge_strategy != MergeStrategy::None
                            });

                            if needs_merge {
                                info!(
                                    run_id = %run.id,
                                    "merge configured, proceeding to merge phase"
                                );
                                // Execute merge phase inline rather than scheduling.
                                // The merge phase is special: it happens after completion
                                // detection but before the run is marked complete.
                                if let Err(e) = execute_merge(&run, &workspace_root) {
                                    // Merge failure fails the run (Section 6).
                                    error!(
                                        run_id = %run.id,
                                        error = %e,
                                        "merge failed"
                                    );
                                    // Write report + summary.json before emitting events.
                                    finalize_run_artifacts(
                                        &storage,
                                        &run,
                                        &config,
                                        ExitReason::Failed,
                                        last_exit_code,
                                        Some(config.completion_mode.as_str()),
                                    )
                                    .await;
                                    let event_payload = EventPayload::RunFailed(RunFailedPayload {
                                        run_id: run.id.clone(),
                                        reason: format!("merge_failed:{e}"),
                                    });
                                    scheduler
                                        .complete_run(
                                            &run.id,
                                            loop_core::RunStatus::Failed,
                                            &event_payload,
                                        )
                                        .await?;
                                    // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                                    maybe_run_postmortem(
                                        &storage,
                                        &run,
                                        &config,
                                        iteration_count,
                                        None,
                                        "merge_failed",
                                    )
                                    .await;
                                    break;
                                }
                                info!(
                                    run_id = %run.id,
                                    "merge completed successfully"
                                );
                            }

                            // Write report + summary.json before emitting events (postmortem-analysis.md Section 5.1).
                            finalize_run_artifacts(
                                &storage,
                                &run,
                                &config,
                                ExitReason::CompletePlan,
                                last_exit_code,
                                Some(config.completion_mode.as_str()),
                            )
                            .await;
                            // Emit RUN_COMPLETED event and update status atomically (Section 4.3).
                            // Complete run BEFORE postmortem to free capacity immediately.
                            let mode = if needs_merge {
                                "merge".to_string()
                            } else {
                                format!("{:?}", config.completion_mode).to_lowercase()
                            };
                            let event_payload = EventPayload::RunCompleted(RunCompletedPayload {
                                run_id: run.id.clone(),
                                mode,
                            });
                            scheduler
                                .complete_run(
                                    &run.id,
                                    loop_core::RunStatus::Completed,
                                    &event_payload,
                                )
                                .await?;
                            // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                            maybe_run_postmortem(
                                &storage,
                                &run,
                                &config,
                                iteration_count,
                                Some(iteration_count),
                                "run_completed",
                            )
                            .await;
                            break;
                        }

                        // Continue to next phase (review or verification).
                    }
                    Err(e) => {
                        // Extract exit code and output tail from enriched error variants.
                        let (fail_exit_code, output_tail) = match &e {
                            RunnerError::ExitCode { code, output_tail } => {
                                (Some(*code), Some(output_tail.as_str()))
                            }
                            RunnerError::TransientApiError { code, output_tail } => {
                                (Some(*code), Some(output_tail.as_str()))
                            }
                            _ => (None, None),
                        };

                        error!(
                            step_id = %step.id,
                            error = %e,
                            exit_code = ?fail_exit_code,
                            output_tail = ?output_tail,
                            "runner execution failed"
                        );

                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, fail_exit_code, None)
                            .await?;

                        // Use the failing step's exit code for finalization artifacts.
                        let exit_code_for_summary = fail_exit_code.unwrap_or(last_exit_code);

                        // Write report + summary.json before emitting events.
                        finalize_run_artifacts(
                            &storage,
                            &run,
                            &config,
                            ExitReason::ClaudeFailed,
                            exit_code_for_summary,
                            Some(config.completion_mode.as_str()),
                        )
                        .await;
                        // Emit RUN_FAILED event and update status atomically (Section 4.3).
                        // Complete run BEFORE postmortem to free capacity immediately.
                        let event_payload = EventPayload::RunFailed(RunFailedPayload {
                            run_id: run.id.clone(),
                            reason: format!("runner_execution_failed:{e}"),
                        });
                        scheduler
                            .complete_run(&run.id, loop_core::RunStatus::Failed, &event_payload)
                            .await?;
                        // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                        maybe_run_postmortem(
                            &storage,
                            &run,
                            &config,
                            iteration_count,
                            None,
                            "claude_failed",
                        )
                        .await;
                        break;
                    }
                }
            }

            StepPhase::Review => {
                // Build review prompt.
                let (prompt, skill_selection, truncation_events, load_failure_events) =
                    build_review_prompt(&run, &config, &discovered_skills);

                // Emit SKILLS_LOAD_FAILED events and increment metrics per Section 4.3 / 7.2.
                for failure in &load_failure_events {
                    skills_metrics.inc_load_failed();
                    let payload = EventPayload::SkillsLoadFailed(SkillsLoadFailedPayload {
                        run_id: run.id.clone(),
                        name: failure.name.clone(),
                        error: failure.error.clone(),
                    });
                    if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                        warn!(
                            run_id = %run.id,
                            error = %e,
                            "failed to emit SKILLS_LOAD_FAILED event"
                        );
                    }
                }

                // Emit SKILLS_SELECTED event (Section 4.3) and log selection decisions (Section 7.1).
                if let Some(ref selection) = skill_selection {
                    // Log selection decisions per Section 7.1.
                    if !selection.skills.is_empty() {
                        info!(
                            run_id = %run.id,
                            step_kind = %selection.step_kind.as_str(),
                            task = %selection.task_label,
                            count = selection.skills.len(),
                            "selected skills for task"
                        );
                        for skill in &selection.skills {
                            info!(
                                run_id = %run.id,
                                skill = %skill.name,
                                reason = %skill.reason,
                                "skill selected"
                            );
                        }
                    }

                    if !selection.errors.is_empty() {
                        warn!(
                            run_id = %run.id,
                            step_kind = %selection.step_kind.as_str(),
                            task = %selection.task_label,
                            errors = ?selection.errors,
                            "skill selection errors"
                        );
                    }

                    // Increment metrics per Section 7.2.
                    skills_metrics.inc_selected(selection.skills.len());

                    let payload = EventPayload::SkillsSelected(SkillsSelectedPayload {
                        run_id: run.id.clone(),
                        step_kind: selection.step_kind.as_str().to_string(),
                        task_label: selection.task_label.clone(),
                        skills: selection
                            .skills
                            .iter()
                            .map(|s| SelectedSkillPayload {
                                name: s.name.clone(),
                                reason: s.reason.clone(),
                            })
                            .collect(),
                        strategy: match selection.strategy {
                            skills::SelectionStrategy::Hint => "hint".to_string(),
                            skills::SelectionStrategy::Match => "match".to_string(),
                            skills::SelectionStrategy::None => "none".to_string(),
                        },
                        errors: selection.errors.clone(),
                    });
                    if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                        warn!(
                            run_id = %run.id,
                            error = %e,
                            "failed to emit SKILLS_SELECTED event"
                        );
                    }
                }

                // Emit SKILLS_TRUNCATED events (Section 4.3) and increment metrics (Section 7.2).
                for event in &truncation_events {
                    skills_metrics.inc_truncated();
                    let payload = EventPayload::SkillsTruncated(SkillsTruncatedPayload {
                        run_id: run.id.clone(),
                        name: event.name.clone(),
                        max_chars: event.max_chars,
                    });
                    if let Err(e) = storage.append_event(&run.id, None, &payload).await {
                        warn!(
                            run_id = %run.id,
                            error = %e,
                            "failed to emit SKILLS_TRUNCATED event"
                        );
                    }
                }

                let prompt_path = run_dir.join("review-prompt.txt");
                std::fs::write(&prompt_path, &prompt)?;

                // Capture HEAD before step for diff stats.
                let head_before = git::get_head_commit(&working_dir).ok();

                // Execute via review runner (may use a different model).
                match review_runner
                    .execute_step(&step, &prompt, &run_dir, &working_dir, cancel_token.clone())
                    .await
                {
                    Ok(result) => {
                        // Log diff stats for this review iteration.
                        if let Some(ref before) = head_before {
                            if let Ok(stats) = git::diff_stats_between(&working_dir, before, "HEAD")
                            {
                                info!(
                                    step_id = %step.id,
                                    phase = "review",
                                    stats = %stats,
                                    "step complete"
                                );
                            }
                        }

                        scheduler
                            .complete_step(
                                &step.id,
                                StepStatus::Succeeded,
                                Some(result.exit_code),
                                Some(result.output_path.to_string_lossy().as_ref()),
                            )
                            .await?;

                        // Emit STEP_FINISHED event.
                        let event_payload = EventPayload::StepFinished(StepFinishedPayload {
                            step_id: step.id.clone(),
                            exit_code: result.exit_code,
                            duration_ms: result.duration_ms,
                            output_path: result.output_path.to_string_lossy().to_string(),
                        });
                        storage
                            .append_event(&run.id, Some(&step.id), &event_payload)
                            .await?;

                        // Persist review output artifacts (mirror if configured).
                        let output_artifacts = mirror_artifact(
                            &run.id,
                            "review_output",
                            &result.output_path,
                            &config.global_log_dir,
                            config.artifact_mode,
                        )?;
                        insert_artifacts(&storage, output_artifacts).await?;

                        let tail_artifacts = mirror_artifact(
                            &run.id,
                            "review_tail",
                            &result.tail_path,
                            &config.global_log_dir,
                            config.artifact_mode,
                        )?;
                        insert_artifacts(&storage, tail_artifacts).await?;

                        // Update consecutive failure counter (reset on success).
                        consecutive_failures.update(StepPhase::Review, StepStatus::Succeeded);

                        // Continue to verification.
                    }
                    Err(e) => {
                        let fail_exit_code = match &e {
                            RunnerError::ExitCode { code, .. }
                            | RunnerError::TransientApiError { code, .. } => Some(*code),
                            _ => None,
                        };

                        error!(
                            step_id = %step.id,
                            error = %e,
                            exit_code = ?fail_exit_code,
                            "review execution failed"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, fail_exit_code, None)
                            .await?;

                        // Update consecutive failure counter.
                        consecutive_failures.update(StepPhase::Review, StepStatus::Failed);

                        // Check threshold and abort if exceeded (consecutive-failure-detection.md Section 5.1).
                        if let Some((phase, count, limit)) =
                            consecutive_failures.check_thresholds(&config)
                        {
                            warn!(
                                run_id = %run.id,
                                phase = %phase.as_str(),
                                count = count,
                                limit = limit,
                                "consecutive failure threshold reached"
                            );
                            finalize_run_artifacts(
                                &storage,
                                &run,
                                &config,
                                ExitReason::Failed,
                                last_exit_code,
                                Some(config.completion_mode.as_str()),
                            )
                            .await;
                            let reason =
                                format!("max_consecutive_failures:{}:{}", phase.as_str(), limit);
                            let event_payload = EventPayload::RunFailed(RunFailedPayload {
                                run_id: run.id.clone(),
                                reason,
                            });
                            scheduler
                                .complete_run(&run.id, loop_core::RunStatus::Failed, &event_payload)
                                .await?;
                            maybe_run_postmortem(
                                &storage,
                                &run,
                                &config,
                                iteration_count,
                                None,
                                "max_consecutive_failures",
                            )
                            .await;
                            break;
                        }
                        // Review failure doesn't fail the run by default; continue to verification.
                    }
                }
            }

            StepPhase::Verification => {
                // Execute verification commands.
                match verifier.execute(&step, &run_dir, &working_dir).await {
                    Ok(result) => {
                        let status = if result.passed {
                            StepStatus::Succeeded
                        } else {
                            StepStatus::Failed
                        };
                        let exit_code = i32::from(!result.passed);

                        scheduler
                            .complete_step(&step.id, status, Some(exit_code), None)
                            .await?;

                        // Emit STEP_FINISHED event for verification (Section 4.3).
                        let event_payload = EventPayload::StepFinished(StepFinishedPayload {
                            step_id: step.id.clone(),
                            exit_code,
                            duration_ms: result.duration_ms,
                            output_path: result
                                .runner_notes_path
                                .as_ref()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        });
                        storage
                            .append_event(&run.id, Some(&step.id), &event_payload)
                            .await?;

                        // Mirror runner notes artifact if present.
                        if let Some(notes_path) = result.runner_notes_path.as_ref() {
                            let note_artifacts = mirror_artifact(
                                &run.id,
                                "runner_notes",
                                notes_path,
                                &config.global_log_dir,
                                config.artifact_mode,
                            )?;
                            insert_artifacts(&storage, note_artifacts).await?;
                        }

                        if result.passed {
                            info!(
                                step_id = %step.id,
                                duration_ms = result.duration_ms,
                                "verification passed"
                            );
                            // Update consecutive failure counter (reset on success).
                            consecutive_failures
                                .update(StepPhase::Verification, StepStatus::Succeeded);
                            // Continue to next iteration.
                        } else {
                            warn!(
                                step_id = %step.id,
                                duration_ms = result.duration_ms,
                                "verification failed, requeuing implementation"
                            );

                            // Update consecutive failure counter.
                            consecutive_failures
                                .update(StepPhase::Verification, StepStatus::Failed);

                            // Check threshold and abort if exceeded (consecutive-failure-detection.md Section 5.1).
                            if let Some((phase, count, limit)) =
                                consecutive_failures.check_thresholds(&config)
                            {
                                warn!(
                                    run_id = %run.id,
                                    phase = %phase.as_str(),
                                    count = count,
                                    limit = limit,
                                    "consecutive failure threshold reached"
                                );
                                // Write report + summary.json before emitting events.
                                finalize_run_artifacts(
                                    &storage,
                                    &run,
                                    &config,
                                    ExitReason::Failed,
                                    last_exit_code,
                                    Some(config.completion_mode.as_str()),
                                )
                                .await;
                                // Emit RUN_FAILED with spec-aligned reason format (Section 4.2).
                                let reason = format!(
                                    "max_consecutive_failures:{}:{}",
                                    phase.as_str(),
                                    limit
                                );
                                let event_payload = EventPayload::RunFailed(RunFailedPayload {
                                    run_id: run.id.clone(),
                                    reason,
                                });
                                scheduler
                                    .complete_run(
                                        &run.id,
                                        loop_core::RunStatus::Failed,
                                        &event_payload,
                                    )
                                    .await?;
                                // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                                maybe_run_postmortem(
                                    &storage,
                                    &run,
                                    &config,
                                    iteration_count,
                                    None,
                                    "max_consecutive_failures",
                                )
                                .await;
                                break;
                            }
                            // Scheduler will requeue implementation on next determine_next_phase.
                        }

                        // Run watchdog evaluation after verification.
                        if let Some(output) = last_output.take() {
                            let mut context =
                                watchdog.detect_signals(&output, &previous_outputs, !result.passed);
                            context.current_rewrite_count = rewrite_count;

                            if context.has_signals() {
                                let decision = watchdog.evaluate(&context);

                                if decision.action == WatchdogAction::Rewrite.as_str() {
                                    let Some(prompt) = last_prompt.as_ref() else {
                                        warn!(run_id = %run.id, "watchdog rewrite skipped (missing prompt)");
                                        previous_outputs.push(output);
                                        continue;
                                    };

                                    let rewrite = watchdog.rewrite_prompt(
                                        &run_dir,
                                        prompt,
                                        decision.signal,
                                        rewrite_count,
                                    )?;
                                    rewrite_count += 1;

                                    // Persist rewritten prompt artifact (mirror if configured).
                                    let rewrite_artifacts = mirror_artifact(
                                        &run.id,
                                        "prompt_rewrite",
                                        &rewrite.prompt_after,
                                        &config.global_log_dir,
                                        config.artifact_mode,
                                    )?;
                                    insert_artifacts(&storage, rewrite_artifacts).await?;

                                    // Emit WATCHDOG_REWRITE event.
                                    let payload =
                                        EventPayload::WatchdogRewrite(WatchdogRewritePayload {
                                            step_id: step.id.clone(),
                                            signal: decision.signal,
                                            prompt_before: rewrite
                                                .prompt_before
                                                .to_string_lossy()
                                                .to_string(),
                                            prompt_after: rewrite
                                                .prompt_after
                                                .to_string_lossy()
                                                .to_string(),
                                        });
                                    storage
                                        .append_event(&run.id, Some(&step.id), &payload)
                                        .await?;

                                    pending_rewrite = Some(rewrite);
                                } else if decision.action == WatchdogAction::Fail.as_str() {
                                    // Write report + summary.json before emitting events.
                                    finalize_run_artifacts(
                                        &storage,
                                        &run,
                                        &config,
                                        ExitReason::Failed,
                                        last_exit_code,
                                        Some(config.completion_mode.as_str()),
                                    )
                                    .await;
                                    let reason = format!("watchdog_failed:{:?}", decision.signal);
                                    let payload = EventPayload::RunFailed(RunFailedPayload {
                                        run_id: run.id.clone(),
                                        reason: reason.clone(),
                                    });
                                    scheduler
                                        .complete_run(
                                            &run.id,
                                            loop_core::RunStatus::Failed,
                                            &payload,
                                        )
                                        .await?;
                                    // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                                    maybe_run_postmortem(
                                        &storage,
                                        &run,
                                        &config,
                                        iteration_count,
                                        None,
                                        "watchdog_failed",
                                    )
                                    .await;
                                    break;
                                }
                            }

                            previous_outputs.push(output);
                        }
                    }
                    Err(e) => {
                        error!(
                            step_id = %step.id,
                            error = %e,
                            "verifier error"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, Some(1), None)
                            .await?;
                        // Continue; scheduler will requeue implementation.
                    }
                }
            }

            StepPhase::Watchdog => {
                // Watchdog phase is triggered by watchdog signals.
                // For now, just mark as succeeded and continue.
                // Full watchdog integration will be added when signals are detected.
                scheduler
                    .complete_step(&step.id, StepStatus::Succeeded, Some(0), None)
                    .await?;
            }

            StepPhase::Merge => {
                // Merge phase - perform git merge if configured.
                // Note: This path is typically not reached because merge is executed
                // inline during completion detection. However, we handle it here for
                // completeness and potential future scheduling changes.
                let workspace_root = PathBuf::from(&run.workspace_root);
                match execute_merge(&run, &workspace_root) {
                    Ok(()) => {
                        info!(
                            run_id = %run.id,
                            "merge phase completed"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Succeeded, Some(0), None)
                            .await?;

                        // Emit STEP_FINISHED event.
                        let event_payload = EventPayload::StepFinished(StepFinishedPayload {
                            step_id: step.id.clone(),
                            exit_code: 0,
                            duration_ms: 0, // Merge is typically fast.
                            output_path: String::new(),
                        });
                        storage
                            .append_event(&run.id, Some(&step.id), &event_payload)
                            .await?;
                    }
                    Err(e) => {
                        error!(
                            run_id = %run.id,
                            error = %e,
                            "merge phase failed"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, Some(1), None)
                            .await?;
                        // Write report + summary.json before emitting events.
                        finalize_run_artifacts(
                            &storage,
                            &run,
                            &config,
                            ExitReason::Failed,
                            last_exit_code,
                            Some(config.completion_mode.as_str()),
                        )
                        .await;
                        // Merge failure fails the run (Section 6).
                        let event_payload = EventPayload::RunFailed(RunFailedPayload {
                            run_id: run.id.clone(),
                            reason: format!("merge_failed:{e}"),
                        });
                        scheduler
                            .complete_run(&run.id, loop_core::RunStatus::Failed, &event_payload)
                            .await?;
                        // Run postmortem analysis (postmortem-analysis.md Section 5.1).
                        maybe_run_postmortem(
                            &storage,
                            &run,
                            &config,
                            iteration_count,
                            None,
                            "merge_phase_failed",
                        )
                        .await;
                        break;
                    }
                }
            }
        }
    }

    // Worktree cleanup (worktrunk-integration.md Section 5.4).
    // Only cleanup if configured and worktree was created.
    // Cleanup failures are logged but do not fail completed runs (Section 6.2).
    let run_id = run.id.clone();
    let run = match storage.get_run(&run_id).await {
        Ok(updated) => updated,
        Err(e) => {
            warn!(
                run_id = %run_id,
                error = %e,
                "failed to reload run before cleanup; using stale state"
            );
            run
        }
    };

    if let Some(ref worktree_config) = run.worktree {
        let defer_cleanup_for_review = run.status == loop_core::RunStatus::Completed
            && run.review_status == ReviewStatus::Pending;

        if defer_cleanup_for_review {
            info!(
                run_id = %run.id,
                worktree_path = %worktree_config.worktree_path,
                "worktree cleanup deferred pending review"
            );
            if let Err(e) = storage
                .update_run_worktree_cleanup(&run.id, "deferred", None)
                .await
            {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "failed to record deferred worktree cleanup"
                );
            }
        } else if config.worktree_cleanup {
            info!(
                run_id = %run.id,
                provider = ?resolved_provider,
                worktree_path = %worktree_config.worktree_path,
                "cleaning up worktree"
            );

            // Create a RunWorktree with the resolved provider for the cleanup call.
            let mut worktree_with_provider = worktree_config.clone();
            worktree_with_provider.provider = resolved_provider;

            match worktree::cleanup(&workspace_root, &worktree_with_provider, &config) {
                Ok(()) => {
                    let cleaned_at = Utc::now().timestamp_millis();
                    if let Err(e) = storage
                        .update_run_worktree_cleanup(&run.id, "cleaned", Some(cleaned_at))
                        .await
                    {
                        warn!(
                            run_id = %run.id,
                            error = %e,
                            "failed to record worktree cleanup status"
                        );
                    }
                    info!(
                        run_id = %run.id,
                        worktree_path = %worktree_config.worktree_path,
                        "worktree removed"
                    );

                    // Emit WORKTREE_REMOVED event (Section 4.3).
                    let removed_event = EventPayload::WorktreeRemoved(WorktreeRemovedPayload {
                        run_id: run.id.clone(),
                        provider: resolved_provider,
                        worktree_path: worktree_config.worktree_path.clone(),
                    });
                    if let Err(e) = storage.append_event(&run.id, None, &removed_event).await {
                        warn!(
                            run_id = %run.id,
                            error = %e,
                            "failed to emit WORKTREE_REMOVED event"
                        );
                    }
                }
                Err(e) => {
                    if let Err(err) = storage
                        .update_run_worktree_cleanup(&run.id, "failed", None)
                        .await
                    {
                        warn!(
                            run_id = %run.id,
                            error = %err,
                            "failed to record worktree cleanup failure"
                        );
                    }
                    // Cleanup failures are logged but do not fail completed runs (Section 6.2).
                    warn!(
                        run_id = %run.id,
                        error = %e,
                        worktree_path = %worktree_config.worktree_path,
                        "worktree cleanup failed (non-fatal)"
                    );
                }
            }
        } else if let Err(e) = storage
            .update_run_worktree_cleanup(&run.id, "skipped", None)
            .await
        {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to record worktree cleanup skip"
            );
        }
    }

    Ok(())
}

/// Execute the merge flow for a completed run.
///
/// Implements spec Section 5.3 Worktree + Merge Flow:
/// 1. Ensure target branch exists (create from base if missing)
/// 2. Merge or squash from `run_branch` into `merge_target_branch`
/// 3. Leave `merge_target_branch` checked out in the primary worktree
///
/// Returns Ok(()) if merge succeeds or is not configured.
/// Returns Err if merge fails (conflicts, dirty tree, etc.).
fn execute_merge(run: &loop_core::Run, workspace_root: &Path) -> Result<(), git::GitError> {
    let Some(worktree) = &run.worktree else {
        // No worktree configured; nothing to merge.
        return Ok(());
    };

    let Some(merge_target) = &worktree.merge_target_branch else {
        // No merge target configured; nothing to merge.
        return Ok(());
    };

    // Skip if strategy is None.
    if worktree.merge_strategy == MergeStrategy::None {
        return Ok(());
    }

    info!(
        run_id = %run.id,
        run_branch = %worktree.run_branch,
        merge_target = %merge_target,
        strategy = ?worktree.merge_strategy,
        "executing merge"
    );

    git::merge_to_target(
        workspace_root,
        &worktree.run_branch,
        merge_target,
        &worktree.base_branch,
        worktree.merge_strategy,
    )
}

async fn insert_artifacts(storage: &Storage, artifacts: Vec<Artifact>) -> AppResult<()> {
    for artifact in artifacts {
        storage.insert_artifact(&artifact).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::Step;

    fn make_step(phase: StepPhase, status: StepStatus) -> Step {
        Step {
            id: Id::new(),
            run_id: Id::new(),
            phase,
            status,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: Some(Utc::now()),
            exit_code: Some(if status == StepStatus::Failed { 1 } else { 0 }),
            prompt_path: None,
            output_path: None,
        }
    }

    #[test]
    fn consecutive_failures_empty_steps() {
        let counters = ConsecutiveFailures::from_steps(&[]);
        assert_eq!(counters.verification, 0);
        assert_eq!(counters.review, 0);
    }

    #[test]
    fn consecutive_failures_counts_verification() {
        let steps = vec![
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
        ];
        let counters = ConsecutiveFailures::from_steps(&steps);
        assert_eq!(counters.verification, 3);
        assert_eq!(counters.review, 0);
    }

    #[test]
    fn consecutive_failures_resets_on_success() {
        let steps = vec![
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Succeeded),
            make_step(StepPhase::Verification, StepStatus::Failed),
        ];
        let counters = ConsecutiveFailures::from_steps(&steps);
        assert_eq!(counters.verification, 1); // Reset after success
    }

    #[test]
    fn consecutive_failures_tracks_both_phases() {
        let steps = vec![
            make_step(StepPhase::Review, StepStatus::Failed),
            make_step(StepPhase::Review, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
            make_step(StepPhase::Verification, StepStatus::Failed),
        ];
        let counters = ConsecutiveFailures::from_steps(&steps);
        assert_eq!(counters.verification, 3);
        assert_eq!(counters.review, 2);
    }

    #[test]
    fn consecutive_failures_ignores_other_phases() {
        let steps = vec![
            make_step(StepPhase::Implementation, StepStatus::Failed),
            make_step(StepPhase::Watchdog, StepStatus::Failed),
            make_step(StepPhase::Merge, StepStatus::Failed),
        ];
        let counters = ConsecutiveFailures::from_steps(&steps);
        assert_eq!(counters.verification, 0);
        assert_eq!(counters.review, 0);
    }

    #[test]
    fn consecutive_failures_update_increments() {
        let mut counters = ConsecutiveFailures::default();
        counters.update(StepPhase::Verification, StepStatus::Failed);
        counters.update(StepPhase::Verification, StepStatus::Failed);
        assert_eq!(counters.verification, 2);

        counters.update(StepPhase::Review, StepStatus::Failed);
        assert_eq!(counters.review, 1);
    }

    #[test]
    fn consecutive_failures_update_resets() {
        let mut counters = ConsecutiveFailures::default();
        counters.verification = 5;
        counters.update(StepPhase::Verification, StepStatus::Succeeded);
        assert_eq!(counters.verification, 0);
    }

    #[test]
    fn consecutive_failures_threshold_exceeded() {
        let mut counters = ConsecutiveFailures::default();
        counters.verification = 3;

        let mut config = Config::default();
        config.max_consecutive_verification_failures = 3;
        config.max_consecutive_review_failures = 0;

        let result = counters.check_thresholds(&config);
        assert!(result.is_some());
        let (phase, count, limit) = result.unwrap();
        assert_eq!(phase, StepPhase::Verification);
        assert_eq!(count, 3);
        assert_eq!(limit, 3);
    }

    #[test]
    fn consecutive_failures_threshold_not_exceeded() {
        let mut counters = ConsecutiveFailures::default();
        counters.verification = 2;

        let mut config = Config::default();
        config.max_consecutive_verification_failures = 3;

        let result = counters.check_thresholds(&config);
        assert!(result.is_none());
    }

    #[test]
    fn consecutive_failures_threshold_disabled() {
        let mut counters = ConsecutiveFailures::default();
        counters.verification = 100;

        let mut config = Config::default();
        config.max_consecutive_verification_failures = 0; // Disabled

        let result = counters.check_thresholds(&config);
        assert!(result.is_none());
    }

    #[test]
    fn consecutive_failures_review_threshold() {
        let mut counters = ConsecutiveFailures::default();
        counters.review = 2;

        let mut config = Config::default();
        config.max_consecutive_verification_failures = 3;
        config.max_consecutive_review_failures = 2;

        let result = counters.check_thresholds(&config);
        assert!(result.is_some());
        let (phase, count, limit) = result.unwrap();
        assert_eq!(phase, StepPhase::Review);
        assert_eq!(count, 2);
        assert_eq!(limit, 2);
    }

    #[test]
    fn remap_to_worktree_rewrites_workspace_path() {
        let result = remap_to_worktree(
            "/home/user/project/specs/foo.md",
            "/home/user/project",
            "/home/user/project.run-abc",
        );
        assert_eq!(result, "/home/user/project.run-abc/specs/foo.md");
    }

    #[test]
    fn remap_to_worktree_preserves_unrelated_path() {
        let result = remap_to_worktree(
            "/tmp/other/file.txt",
            "/home/user/project",
            "/home/user/project.run-abc",
        );
        assert_eq!(result, "/tmp/other/file.txt");
    }

    #[test]
    fn remap_to_worktree_handles_trailing_slash() {
        let result = remap_to_worktree(
            "/home/user/project/specs/plan.md",
            "/home/user/project/",
            "/home/user/project.run-abc",
        );
        assert_eq!(result, "/home/user/project.run-abc/specs/plan.md");
    }
}
