//! loopd - Agent Loop Orchestrator Daemon
//!
//! Library components for the daemon process.
//! See spec: specs/orchestrator-daemon.md

pub mod git;
pub mod naming;
pub mod runner;
pub mod scheduler;
pub mod server;
pub mod storage;
pub mod verifier;
pub mod watchdog;
pub mod worktree;
pub mod worktree_worktrunk;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_core::completion::check_completion;
use loop_core::events::{
    EventPayload, RunCompletedPayload, RunFailedPayload, StepFinishedPayload, StepStartedPayload,
    WatchdogRewritePayload, WorktreeProviderSelectedPayload,
};
use loop_core::types::MergeStrategy;
use loop_core::{
    mirror_artifact, write_and_mirror_artifact, Artifact, Config, Id, StepPhase, StepStatus,
};
use runner::{Runner, RunnerConfig};
use scheduler::Scheduler;
use storage::Storage;
use tracing::{error, info, warn};
use verifier::{Verifier, VerifierConfig};
use watchdog::{Watchdog, WatchdogAction};

/// Daemon configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Path to the SQLite database.
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
            max_runs_per_workspace: None,
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

/// Build the implementation prompt with context file references.
/// Matches bin/loop behavior: @spec @plan @runner-notes @LEARNINGS.md
fn build_implementation_prompt(run: &loop_core::Run, run_dir: &Path, config: &Config) -> String {
    let mut refs = format!("@{}", run.spec_path);

    if let Some(plan_path) = &run.plan_path {
        refs.push_str(&format!(" @{}", plan_path));
    }

    // Add runner notes (created by verifier on failure)
    let runner_notes = run_dir.join("runner-notes.txt");
    refs.push_str(&format!(" @{}", runner_notes.display()));

    // Add learnings file if it exists
    let workspace_root = PathBuf::from(&run.workspace_root);
    let learnings_path = workspace_root.join(&config.specs_dir).join("LEARNINGS.md");
    if learnings_path.exists() {
        refs.push_str(&format!(" @{}", learnings_path.display()));
    }

    let completion_note = match config.completion_mode {
        loop_core::CompletionMode::Exact => {
            "The runner detects completion only if your entire output is exactly <promise>COMPLETE</promise>."
        }
        loop_core::CompletionMode::Trailing => {
            "The runner detects completion when the last non-empty line is exactly <promise>COMPLETE</promise>."
        }
    };

    format!(
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
5. Make exactly one git commit for your changes using `gritty commit --accept`.
6. If a task is blocked by a production bug or missing test infrastructure, mark it `[~]` and add it to
   the plan's `## Blockers Discovered` section. Do not mark it `[x]`.
7. If (and only if) all `[ ]` and `[~]` tasks in the plan are complete (ignore `[ ]?` manual QA items), respond with:
<promise>COMPLETE</promise>

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
    )
}

/// Build the review prompt.
/// Matches bin/loop's load_reviewer_prompt behavior.
fn build_review_prompt(run: &loop_core::Run) -> String {
    let mut refs = format!("@{}", run.spec_path);
    if let Some(plan_path) = &run.plan_path {
        refs.push_str(&format!(" @{}", plan_path));
    }

    format!(
        r#"{refs}

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
- Do not approve until issues are resolved"#
    )
}

/// Process a single run through all phases.
///
/// Implements the main flow from spec Section 5.1:
/// implementation -> review -> verification -> (watchdog if signals) -> completion
async fn process_run(
    scheduler: Arc<Scheduler>,
    storage: Arc<Storage>,
    run: loop_core::Run,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("processing run: {} ({})", run.name, run.id);

    // Parse run config.
    let config: Config = run
        .config_json
        .as_ref()
        .map(|json| serde_json::from_str(json))
        .transpose()?
        .unwrap_or_default();

    // Resolve worktree provider and emit event (worktrunk-integration.md Section 5.2).
    let workspace_root_path = std::path::Path::new(&run.workspace_root);
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

    // Set up paths.
    let workspace_root = PathBuf::from(&run.workspace_root);
    let run_dir = run_dir(&workspace_root, &run.id);
    std::fs::create_dir_all(&run_dir)?;

    // Determine working directory (worktree if configured, else workspace root).
    let working_dir = run
        .worktree
        .as_ref()
        .map(|wt| PathBuf::from(&wt.worktree_path))
        .unwrap_or_else(|| workspace_root.clone());

    // Create runner and verifier from config.
    let runner = Runner::new(RunnerConfig::from_config(&config));
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
            // Emit RUN_FAILED event (Section 4.3).
            let event_payload = EventPayload::RunFailed(RunFailedPayload {
                run_id: run.id.clone(),
                reason: format!("iteration_limit_reached:{}", config.iterations),
            });
            storage.append_event(&run.id, None, &event_payload).await?;
            scheduler
                .release_run(&run.id, loop_core::RunStatus::Failed)
                .await?;
            break;
        }

        // Determine the next phase.
        let next_phase = scheduler.determine_next_phase(&run.id).await?;

        let Some(phase) = next_phase else {
            // No more phases; run is complete (merge was terminal).
            info!("run complete: {}", run.id);
            // Emit RUN_COMPLETED event (Section 4.3).
            let event_payload = EventPayload::RunCompleted(RunCompletedPayload {
                run_id: run.id.clone(),
                mode: "merge".to_string(),
            });
            storage.append_event(&run.id, None, &event_payload).await?;
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
                    let prompt = build_implementation_prompt(&run, &run_dir, &config);
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

                // Execute via runner.
                match runner
                    .execute_step(&step, &prompt, &run_dir, &working_dir)
                    .await
                {
                    Ok(result) => {
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
                            let needs_merge = run
                                .worktree
                                .as_ref()
                                .map(|wt| {
                                    wt.merge_target_branch.is_some()
                                        && wt.merge_strategy != MergeStrategy::None
                                })
                                .unwrap_or(false);

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
                                    let event_payload = EventPayload::RunFailed(RunFailedPayload {
                                        run_id: run.id.clone(),
                                        reason: format!("merge_failed:{}", e),
                                    });
                                    storage.append_event(&run.id, None, &event_payload).await?;
                                    scheduler
                                        .release_run(&run.id, loop_core::RunStatus::Failed)
                                        .await?;
                                    break;
                                }
                                info!(
                                    run_id = %run.id,
                                    "merge completed successfully"
                                );
                            }

                            // Emit RUN_COMPLETED event (Section 4.3).
                            let mode = if needs_merge {
                                "merge".to_string()
                            } else {
                                format!("{:?}", config.completion_mode).to_lowercase()
                            };
                            let event_payload = EventPayload::RunCompleted(RunCompletedPayload {
                                run_id: run.id.clone(),
                                mode,
                            });
                            storage.append_event(&run.id, None, &event_payload).await?;
                            scheduler
                                .release_run(&run.id, loop_core::RunStatus::Completed)
                                .await?;
                            break;
                        }

                        // Continue to next phase (review or verification).
                    }
                    Err(e) => {
                        error!(
                            step_id = %step.id,
                            error = %e,
                            "runner execution failed"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, None, None)
                            .await?;
                        // Emit RUN_FAILED event (Section 4.3).
                        let event_payload = EventPayload::RunFailed(RunFailedPayload {
                            run_id: run.id.clone(),
                            reason: format!("runner_execution_failed:{}", e),
                        });
                        storage.append_event(&run.id, None, &event_payload).await?;
                        scheduler
                            .release_run(&run.id, loop_core::RunStatus::Failed)
                            .await?;
                        break;
                    }
                }
            }

            StepPhase::Review => {
                // Build review prompt.
                let prompt = build_review_prompt(&run);
                let prompt_path = run_dir.join("review-prompt.txt");
                std::fs::write(&prompt_path, &prompt)?;

                // Execute via runner.
                match runner
                    .execute_step(&step, &prompt, &run_dir, &working_dir)
                    .await
                {
                    Ok(result) => {
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

                        // Continue to verification.
                    }
                    Err(e) => {
                        error!(
                            step_id = %step.id,
                            error = %e,
                            "review execution failed"
                        );
                        scheduler
                            .complete_step(&step.id, StepStatus::Failed, None, None)
                            .await?;
                        // Review failure doesn't fail the run; continue to verification.
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
                        let exit_code = if result.passed { 0 } else { 1 };

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
                            // Continue to next iteration.
                        } else {
                            warn!(
                                step_id = %step.id,
                                duration_ms = result.duration_ms,
                                "verification failed, requeuing implementation"
                            );
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
                                    let reason = format!("watchdog_failed:{:?}", decision.signal);
                                    let payload = EventPayload::RunFailed(RunFailedPayload {
                                        run_id: run.id.clone(),
                                        reason: reason.clone(),
                                    });
                                    storage.append_event(&run.id, None, &payload).await?;
                                    scheduler
                                        .release_run(&run.id, loop_core::RunStatus::Failed)
                                        .await?;
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
                        // Merge failure fails the run (Section 6).
                        let event_payload = EventPayload::RunFailed(RunFailedPayload {
                            run_id: run.id.clone(),
                            reason: format!("merge_failed:{}", e),
                        });
                        storage.append_event(&run.id, None, &event_payload).await?;
                        scheduler
                            .release_run(&run.id, loop_core::RunStatus::Failed)
                            .await?;
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Execute the merge flow for a completed run.
///
/// Implements spec Section 5.3 Worktree + Merge Flow:
/// 1. Ensure target branch exists (create from base if missing)
/// 2. Merge or squash from run_branch into merge_target_branch
/// 3. Leave merge_target_branch checked out in the primary worktree
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

async fn insert_artifacts(
    storage: &Storage,
    artifacts: Vec<Artifact>,
) -> Result<(), Box<dyn std::error::Error>> {
    for artifact in artifacts {
        storage.insert_artifact(&artifact).await?;
    }
    Ok(())
}
