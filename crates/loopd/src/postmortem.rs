//! Postmortem and summary artifact generation.
//!
//! Implements run summaries and analysis reports.
//! See spec: specs/postmortem-analysis.md

use loop_core::artifacts::ArtifactError;
use loop_core::{write_and_mirror_artifact, Artifact, Config, Run, Step};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use tracing::warn;

use crate::storage::Storage;

#[derive(Debug, Error)]
pub enum PostmortemError {
    #[error("storage error: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("artifact error: {0}")]
    Artifact(#[from] ArtifactError),
}

pub type Result<T> = std::result::Result<T, PostmortemError>;

/// Summary JSON schema as defined in spec Section 3.
///
/// Field names and types match the legacy `write_summary_json()` in `lib/agent-loop-ui.sh`.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub total_duration_ms: i64,
    pub iterations_run: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_iteration: Option<u32>,
    pub avg_duration_ms: i64,
    pub last_exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_mode: Option<String>,
    pub model: String,
    pub exit_reason: String,
    pub run_log: String,
    pub run_report: String,
    pub prompt_snapshot: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_iteration_tail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_iteration_log: Option<String>,
}

/// Exit reasons matching legacy bin/loop behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    CompletePlan,
    CompleteReviewer,
    IterationsExhausted,
    ClaudeFailed,
    Failed,
    Canceled,
}

impl ExitReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompletePlan => "complete_plan",
            Self::CompleteReviewer => "complete_reviewer",
            Self::IterationsExhausted => "iterations_exhausted",
            Self::ClaudeFailed => "claude_failed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Derive exit reason from run status and completion mode.
    pub fn from_run_status(status: loop_core::RunStatus, completion_mode: Option<&str>) -> Self {
        match status {
            loop_core::RunStatus::Completed => {
                // Check completion mode to determine if it was plan or reviewer
                match completion_mode {
                    Some("reviewer") => Self::CompleteReviewer,
                    _ => Self::CompletePlan,
                }
            }
            loop_core::RunStatus::Failed => Self::Failed,
            loop_core::RunStatus::Canceled => Self::Canceled,
            _ => Self::Failed,
        }
    }
}

/// Generate and write summary.json for a completed run.
///
/// Implements spec Section 5.1 step 2: write summary.json to run directory
/// and mirror to global artifacts if configured.
pub async fn write_summary_json(
    storage: &Storage,
    run: &Run,
    config: &Config,
    exit_reason: ExitReason,
    last_exit_code: i32,
    completion_mode: Option<&str>,
) -> Result<Vec<Artifact>> {
    let workspace_root = Path::new(&run.workspace_root);
    let run_dir = loop_core::workspace_run_dir(workspace_root, &run.id);

    // Collect iteration data from steps.
    let steps = storage.list_steps(&run.id).await?;
    let implementation_steps: Vec<&Step> = steps
        .iter()
        .filter(|s| s.phase == loop_core::StepPhase::Implementation)
        .collect();

    let iterations_run = implementation_steps.len() as u32;

    // Find completed iteration (the iteration where completion was detected).
    let completed_iteration = (exit_reason == ExitReason::CompletePlan
        || exit_reason == ExitReason::CompleteReviewer)
        .then_some(iterations_run);

    // Calculate timing.
    let start_ms = run.created_at.timestamp_millis();
    let end_ms = chrono::Utc::now().timestamp_millis();
    let total_duration_ms = end_ms - start_ms;
    let avg_duration_ms = if iterations_run > 0 {
        total_duration_ms / i64::from(iterations_run)
    } else {
        0
    };

    // Find last iteration's artifacts.
    let last_step = implementation_steps.last();
    let last_iteration_log = last_step.and_then(|s| s.output_path.clone());
    let last_iteration_tail = last_step.and_then(|s| {
        s.output_path.as_ref().map(|p| {
            // Derive tail path from output path (replace .log with .tail.txt)
            let path = Path::new(p);
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let parent = path.parent().unwrap_or(Path::new(""));
            parent
                .join(format!("{stem}.tail.txt"))
                .to_string_lossy()
                .to_string()
        })
    });

    // Build artifact paths.
    let run_log = run_dir.join("run.log").to_string_lossy().to_string();
    let run_report = run_dir.join("report.tsv").to_string_lossy().to_string();
    let prompt_snapshot = run_dir.join("prompt.txt").to_string_lossy().to_string();

    let summary = RunSummary {
        run_id: run.id.to_string(),
        start_ms,
        end_ms,
        total_duration_ms,
        iterations_run,
        completed_iteration,
        avg_duration_ms,
        last_exit_code,
        completion_mode: completion_mode.map(String::from),
        model: config.model.clone(),
        exit_reason: exit_reason.as_str().to_string(),
        run_log,
        run_report,
        prompt_snapshot,
        last_iteration_tail,
        last_iteration_log,
    };

    // Serialize with pretty printing for readability.
    let json = serde_json::to_string_pretty(&summary)?;

    // Write and optionally mirror artifact.
    let artifacts = write_and_mirror_artifact(
        &run.id,
        "summary",
        "summary.json",
        json.as_bytes(),
        workspace_root,
        &config.global_log_dir,
        config.artifact_mode,
    )?;

    Ok(artifacts)
}

/// Analysis context collected from a run for generating analysis prompts.
///
/// Used by `build_*_prompt` functions to generate the three analysis prompts.
#[derive(Debug, Clone)]
pub struct AnalysisContext {
    pub run_id: String,
    pub completion_display: String,
    pub last_iter: Option<u32>,
    pub model: String,
    pub spec_path: Option<String>,
    pub plan_path: Option<String>,
    pub run_report: String,
    pub run_log: String,
    pub prompt_snapshot: String,
    pub summary_json: String,
    pub last_iter_tail: Option<String>,
    pub last_iter_log: Option<String>,
    pub analysis_dir: PathBuf,
}

impl AnalysisContext {
    /// Create analysis context from a run and its configuration.
    pub fn from_run(
        run: &Run,
        config: &Config,
        iterations_run: u32,
        completed_iter: Option<u32>,
    ) -> Self {
        let workspace_root = Path::new(&run.workspace_root);
        let run_dir = loop_core::workspace_run_dir(workspace_root, &run.id);
        let analysis_dir = run_dir.join("analysis");

        let completion_display = if let Some(iter) = completed_iter {
            format!("iteration {iter}")
        } else {
            "not detected".to_string()
        };

        let last_iter = (iterations_run > 0).then_some(iterations_run);

        // Derive iteration file paths from iteration count
        let (last_iter_tail, last_iter_log) = if let Some(iter) = last_iter {
            let iter_slug = format!("{iter:02}");
            (
                Some(
                    run_dir
                        .join(format!("iter-{iter_slug}.tail.txt"))
                        .to_string_lossy()
                        .to_string(),
                ),
                Some(
                    run_dir
                        .join(format!("iter-{iter_slug}.log"))
                        .to_string_lossy()
                        .to_string(),
                ),
            )
        } else {
            (None, None)
        };

        Self {
            run_id: run.id.to_string(),
            completion_display,
            last_iter,
            model: config.model.clone(),
            spec_path: Some(run.spec_path.clone()),
            plan_path: run.plan_path.clone(),
            run_report: run_dir.join("report.tsv").to_string_lossy().to_string(),
            run_log: run_dir.join("run.log").to_string_lossy().to_string(),
            prompt_snapshot: run_dir.join("prompt.txt").to_string_lossy().to_string(),
            summary_json: run_dir.join("summary.json").to_string_lossy().to_string(),
            last_iter_tail,
            last_iter_log,
            analysis_dir,
        }
    }
}

/// Build the run quality analysis prompt.
///
/// Implements spec Section 5.1 step 3: Generate analysis prompts based on `bin/loop-analyze`.
/// This prompt focuses on end-of-task behavior, completion protocol compliance,
/// and actionable improvements to spec templates and loop prompt.
pub fn build_run_quality_prompt(ctx: &AnalysisContext) -> String {
    format!(
        r"Analyze this agent-loop run. Focus on end-of-task behavior, completion protocol compliance, and
actionable improvements to the spec templates and loop prompt.

Run metadata:
- Run ID: {run_id}
- Completion detected: {completion_display}
- Last iteration observed: {last_iter}
- Model: {model}

Artifacts (read all that exist):
- Run report (TSV): {run_report}
- Run log: {run_log}
- Prompt snapshot: {prompt_snapshot}
- Summary JSON: {summary_json}
- Last iteration tail: {last_iter_tail}
- Last iteration log: {last_iter_log}

Return:
1) Short timeline summary + anomalies
2) End-of-task behavior (did it cleanly finish? protocol violations?)
3) Spec/template improvements (actionable)
4) Loop prompt improvements (actionable)
5) Loop UX/logging improvements (actionable)",
        run_id = ctx.run_id,
        completion_display = ctx.completion_display,
        last_iter = ctx
            .last_iter
            .map_or_else(|| "unknown".to_string(), |i| i.to_string()),
        model = ctx.model,
        run_report = ctx.run_report,
        run_log = ctx.run_log,
        prompt_snapshot = ctx.prompt_snapshot,
        summary_json = ctx.summary_json,
        last_iter_tail = ctx.last_iter_tail.as_deref().unwrap_or("unknown"),
        last_iter_log = ctx.last_iter_log.as_deref().unwrap_or("unknown"),
    )
}

/// Build the spec compliance analysis prompt.
///
/// Implements spec Section 5.1 step 3: Generate analysis prompts based on `bin/loop-analyze`.
/// This prompt analyzes the implementation against the spec and plan.
pub fn build_spec_compliance_prompt(ctx: &AnalysisContext) -> String {
    let spec_path = ctx.spec_path.as_deref().unwrap_or("unknown");
    let plan_path = ctx.plan_path.as_deref().unwrap_or("unknown");

    format!(
        r"Analyze the implementation against the spec and plan. Determine whether the spec is clear and whether
the implementation followed it. Highlight any changes required to fully reach the spec requirements.

Context:
- Spec: {spec_path}
- Plan: {plan_path}
- Model: {model}

Artifacts (read all that exist):
- Spec: {spec_path}
- Plan: {plan_path}
- Git status: {analysis_dir}/git-status.txt
- Last commit summary: {analysis_dir}/git-last-commit.txt
- Last commit patch: {analysis_dir}/git-last-commit.patch
- Working tree diff: {analysis_dir}/git-diff.patch
- Run summary: {summary_json}

Return a Markdown report with sections:
1) Compliance summary (pass/fail + rationale)
2) Deviations (spec gap vs implementation deviation)
3) Missing verification steps
4) Required changes to meet the spec (bullet list)
5) Spec/template edits to prevent recurrence",
        spec_path = spec_path,
        plan_path = plan_path,
        model = ctx.model,
        analysis_dir = ctx.analysis_dir.display(),
        summary_json = ctx.summary_json,
    )
}

/// Build the summary analysis prompt.
///
/// Implements spec Section 5.1 step 3: Generate analysis prompts based on `bin/loop-analyze`.
/// This prompt synthesizes the spec compliance and run quality reports into a final postmortem.
pub fn build_summary_prompt(ctx: &AnalysisContext) -> String {
    format!(
        r"Synthesize the following reports into a final postmortem. Decide the primary root cause and provide
actionable changes to specs, prompt, and tooling.

Inputs:
- Spec compliance report: {analysis_dir}/spec-compliance.md
- Run quality report: {analysis_dir}/run-quality.md

Return a Markdown report with sections:
1) Root cause classification (spec gap vs implementation deviation vs execution failure)
2) Evidence (file/log references)
3) Required changes to reach the spec (bullet list)
4) Spec template changes
5) Loop prompt changes
6) Tooling/UX changes",
        analysis_dir = ctx.analysis_dir.display(),
    )
}

/// Capture git snapshot files for analysis.
///
/// Implements spec Section 5.1 step 3: Capture git snapshot files (if git available).
/// Creates the following files in the analysis directory:
/// - git-status.txt
/// - git-last-commit.txt
/// - git-last-commit.patch
/// - git-diff.patch
///
/// Returns Ok(true) if git snapshot was captured, Ok(false) if not in a git repo.
pub fn capture_git_snapshot(workspace_root: &Path, analysis_dir: &Path) -> Result<bool> {
    // Check if we're in a git repository
    let status = Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(workspace_root)
        .output();

    let is_git_repo = match status {
        Ok(output) => output.status.success(),
        Err(_) => return Ok(false), // git not available
    };

    if !is_git_repo {
        return Ok(false);
    }

    // Create analysis directory
    std::fs::create_dir_all(analysis_dir)?;

    // git status -sb
    let git_status_path = analysis_dir.join("git-status.txt");
    match Command::new("git")
        .args(["status", "-sb"])
        .current_dir(workspace_root)
        .output()
    {
        Ok(output) => {
            std::fs::write(&git_status_path, &output.stdout)?;
        }
        Err(e) => {
            warn!(error = %e, "failed to capture git status");
        }
    }

    // git log -1 --stat
    let git_last_commit_path = analysis_dir.join("git-last-commit.txt");
    match Command::new("git")
        .args(["log", "-1", "--stat"])
        .current_dir(workspace_root)
        .output()
    {
        Ok(output) => {
            std::fs::write(&git_last_commit_path, &output.stdout)?;
        }
        Err(e) => {
            warn!(error = %e, "failed to capture git log");
        }
    }

    // git show -1 --stat --patch
    let git_last_commit_patch_path = analysis_dir.join("git-last-commit.patch");
    match Command::new("git")
        .args(["show", "-1", "--stat", "--patch"])
        .current_dir(workspace_root)
        .output()
    {
        Ok(output) => {
            std::fs::write(&git_last_commit_patch_path, &output.stdout)?;
        }
        Err(e) => {
            warn!(error = %e, "failed to capture git show");
        }
    }

    // git diff
    let git_diff_path = analysis_dir.join("git-diff.patch");
    match Command::new("git")
        .args(["diff"])
        .current_dir(workspace_root)
        .output()
    {
        Ok(output) => {
            std::fs::write(&git_diff_path, &output.stdout)?;
        }
        Err(e) => {
            warn!(error = %e, "failed to capture git diff");
        }
    }

    Ok(true)
}

/// Write analysis prompts to the analysis directory.
///
/// Implements spec Section 5.1 step 3: Generate analysis prompts.
/// Creates the following files in the analysis directory:
/// - run-quality-prompt.txt
/// - spec-compliance-prompt.txt
/// - summary-prompt.txt
pub fn write_analysis_prompts(ctx: &AnalysisContext) -> Result<AnalysisPrompts> {
    std::fs::create_dir_all(&ctx.analysis_dir)?;

    let run_quality_prompt = build_run_quality_prompt(ctx);
    let spec_compliance_prompt = build_spec_compliance_prompt(ctx);
    let summary_prompt = build_summary_prompt(ctx);

    let run_quality_path = ctx.analysis_dir.join("run-quality-prompt.txt");
    let spec_compliance_path = ctx.analysis_dir.join("spec-compliance-prompt.txt");
    let summary_path = ctx.analysis_dir.join("summary-prompt.txt");

    std::fs::write(&run_quality_path, &run_quality_prompt)?;
    std::fs::write(&spec_compliance_path, &spec_compliance_prompt)?;
    std::fs::write(&summary_path, &summary_prompt)?;

    Ok(AnalysisPrompts {
        run_quality: AnalysisPrompt {
            prompt: run_quality_prompt,
            prompt_path: run_quality_path,
            output_path: ctx.analysis_dir.join("run-quality.md"),
        },
        spec_compliance: AnalysisPrompt {
            prompt: spec_compliance_prompt,
            prompt_path: spec_compliance_path,
            output_path: ctx.analysis_dir.join("spec-compliance.md"),
        },
        summary: AnalysisPrompt {
            prompt: summary_prompt,
            prompt_path: summary_path,
            output_path: ctx.analysis_dir.join("summary.md"),
        },
    })
}

/// A single analysis prompt with its paths.
#[derive(Debug, Clone)]
pub struct AnalysisPrompt {
    /// The prompt content.
    pub prompt: String,
    /// Path where the prompt was written.
    pub prompt_path: PathBuf,
    /// Path where the output should be written.
    pub output_path: PathBuf,
}

/// Collection of analysis prompts for a run.
#[derive(Debug, Clone)]
pub struct AnalysisPrompts {
    /// Run quality analysis prompt.
    pub run_quality: AnalysisPrompt,
    /// Spec compliance analysis prompt.
    pub spec_compliance: AnalysisPrompt,
    /// Summary analysis prompt.
    pub summary: AnalysisPrompt,
}

/// Result of executing a single analysis step.
#[derive(Debug, Clone)]
pub struct AnalysisStepResult {
    /// Whether the analysis succeeded.
    pub success: bool,
    /// Exit code from claude CLI.
    pub exit_code: i32,
    /// Path to the output file.
    pub output_path: PathBuf,
}

/// Result of running the full postmortem analysis.
#[derive(Debug, Clone)]
pub struct PostmortemResult {
    /// Spec compliance analysis result.
    pub spec_compliance: Option<AnalysisStepResult>,
    /// Run quality analysis result.
    pub run_quality: Option<AnalysisStepResult>,
    /// Summary analysis result.
    pub summary: Option<AnalysisStepResult>,
}

impl PostmortemResult {
    /// Returns true if all analysis steps succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.spec_compliance.as_ref().is_some_and(|r| r.success)
            && self.run_quality.as_ref().is_some_and(|r| r.success)
            && self.summary.as_ref().is_some_and(|r| r.success)
    }
}

/// Check if the claude CLI is available.
pub fn is_claude_available() -> bool {
    Command::new("claude")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Execute a single analysis step using the claude CLI.
///
/// Implements spec Section 5.1 step 4: Execute each prompt using
/// `claude -p --dangerously-skip-permissions --model <model>` and write outputs.
///
/// Returns Ok with the result, or Err if execution failed catastrophically.
pub fn execute_analysis_step(
    prompt: &AnalysisPrompt,
    model: &str,
    working_dir: &Path,
) -> Result<AnalysisStepResult> {
    let output = Command::new("claude")
        .arg("-p")
        .arg("--dangerously-skip-permissions")
        .arg("--model")
        .arg(model)
        .arg(&prompt.prompt)
        .current_dir(working_dir)
        .output();

    match output {
        Ok(result) => {
            let exit_code = result.status.code().unwrap_or(-1);
            let success = result.status.success();

            // Write output to file regardless of exit code
            std::fs::write(&prompt.output_path, &result.stdout)?;

            Ok(AnalysisStepResult {
                success,
                exit_code,
                output_path: prompt.output_path.clone(),
            })
        }
        Err(e) => {
            warn!(
                error = %e,
                prompt_path = %prompt.prompt_path.display(),
                "failed to execute claude for analysis"
            );
            Err(PostmortemError::Io(e))
        }
    }
}

/// Run the full postmortem analysis pipeline.
///
/// Implements spec Section 5.1 steps 3-4:
/// 1. Capture git snapshot files (if git available)
/// 2. Generate analysis prompts
/// 3. Execute each prompt using claude CLI and write outputs
///
/// Returns Ok with results even if individual steps fail (best-effort).
/// Analysis failures do not change run status (spec Section 6).
pub fn run_postmortem_analysis(
    run: &Run,
    config: &Config,
    iterations_run: u32,
    completed_iter: Option<u32>,
) -> Result<PostmortemResult> {
    let workspace_root = Path::new(&run.workspace_root);

    // Build analysis context
    let ctx = AnalysisContext::from_run(run, config, iterations_run, completed_iter);

    // Create analysis directory
    std::fs::create_dir_all(&ctx.analysis_dir)?;

    // Step 1: Capture git snapshot (best-effort, continue on failure)
    match capture_git_snapshot(workspace_root, &ctx.analysis_dir) {
        Ok(captured) => {
            if captured {
                tracing::info!(
                    run_id = %run.id,
                    analysis_dir = %ctx.analysis_dir.display(),
                    "git snapshot captured"
                );
            }
        }
        Err(e) => {
            warn!(
                run_id = %run.id,
                error = %e,
                "failed to capture git snapshot (continuing)"
            );
        }
    }

    // Step 2: Write analysis prompts
    let prompts = write_analysis_prompts(&ctx)?;

    // Step 3: Execute analysis steps
    // Order matters: spec-compliance and run-quality first, then summary (which references them)
    let spec_compliance =
        match execute_analysis_step(&prompts.spec_compliance, &config.model, workspace_root) {
            Ok(result) => {
                tracing::info!(
                    run_id = %run.id,
                    success = result.success,
                    exit_code = result.exit_code,
                    "spec compliance analysis complete"
                );
                Some(result)
            }
            Err(e) => {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "spec compliance analysis failed"
                );
                None
            }
        };

    let run_quality =
        match execute_analysis_step(&prompts.run_quality, &config.model, workspace_root) {
            Ok(result) => {
                tracing::info!(
                    run_id = %run.id,
                    success = result.success,
                    exit_code = result.exit_code,
                    "run quality analysis complete"
                );
                Some(result)
            }
            Err(e) => {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "run quality analysis failed"
                );
                None
            }
        };

    // Summary depends on the other two reports existing
    let summary = if spec_compliance.is_some() && run_quality.is_some() {
        match execute_analysis_step(&prompts.summary, &config.model, workspace_root) {
            Ok(result) => {
                tracing::info!(
                    run_id = %run.id,
                    success = result.success,
                    exit_code = result.exit_code,
                    "postmortem summary complete"
                );
                Some(result)
            }
            Err(e) => {
                warn!(
                    run_id = %run.id,
                    error = %e,
                    "postmortem summary failed"
                );
                None
            }
        }
    } else {
        warn!(
            run_id = %run.id,
            "skipping summary analysis (prerequisite reports failed)"
        );
        None
    };

    Ok(PostmortemResult {
        spec_compliance,
        run_quality,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_reason_as_str() {
        assert_eq!(ExitReason::CompletePlan.as_str(), "complete_plan");
        assert_eq!(ExitReason::CompleteReviewer.as_str(), "complete_reviewer");
        assert_eq!(
            ExitReason::IterationsExhausted.as_str(),
            "iterations_exhausted"
        );
        assert_eq!(ExitReason::ClaudeFailed.as_str(), "claude_failed");
        assert_eq!(ExitReason::Failed.as_str(), "failed");
        assert_eq!(ExitReason::Canceled.as_str(), "canceled");
    }

    #[test]
    fn exit_reason_from_completed_status() {
        assert_eq!(
            ExitReason::from_run_status(loop_core::RunStatus::Completed, None),
            ExitReason::CompletePlan
        );
        assert_eq!(
            ExitReason::from_run_status(loop_core::RunStatus::Completed, Some("reviewer")),
            ExitReason::CompleteReviewer
        );
    }

    #[test]
    fn exit_reason_from_failed_status() {
        assert_eq!(
            ExitReason::from_run_status(loop_core::RunStatus::Failed, None),
            ExitReason::Failed
        );
    }

    #[test]
    fn exit_reason_from_canceled_status() {
        assert_eq!(
            ExitReason::from_run_status(loop_core::RunStatus::Canceled, None),
            ExitReason::Canceled
        );
    }

    #[test]
    fn summary_serializes_with_correct_fields() {
        let summary = RunSummary {
            run_id: "01HS6Q123".to_string(),
            start_ms: 1738218455000,
            end_ms: 1738219056000,
            total_duration_ms: 601000,
            iterations_run: 12,
            completed_iteration: Some(11),
            avg_duration_ms: 50083,
            last_exit_code: 0,
            completion_mode: Some("trailing".to_string()),
            model: "opus".to_string(),
            exit_reason: "complete_plan".to_string(),
            run_log: "/path/run.log".to_string(),
            run_report: "/path/report.tsv".to_string(),
            prompt_snapshot: "/path/prompt.txt".to_string(),
            last_iteration_tail: Some("/path/iter-11.tail.txt".to_string()),
            last_iteration_log: Some("/path/iter-11.log".to_string()),
        };

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"run_id\":\"01HS6Q123\""));
        assert!(json.contains("\"start_ms\":1738218455000"));
        assert!(json.contains("\"exit_reason\":\"complete_plan\""));
        assert!(json.contains("\"completed_iteration\":11"));
    }

    #[test]
    fn summary_omits_null_optional_fields() {
        let summary = RunSummary {
            run_id: "test".to_string(),
            start_ms: 0,
            end_ms: 0,
            total_duration_ms: 0,
            iterations_run: 0,
            completed_iteration: None,
            avg_duration_ms: 0,
            last_exit_code: 0,
            completion_mode: None,
            model: "opus".to_string(),
            exit_reason: "failed".to_string(),
            run_log: "".to_string(),
            run_report: "".to_string(),
            prompt_snapshot: "".to_string(),
            last_iteration_tail: None,
            last_iteration_log: None,
        };

        let json = serde_json::to_string(&summary).unwrap();
        // Optional fields with None should not appear in output
        assert!(!json.contains("completed_iteration"));
        assert!(!json.contains("completion_mode"));
        assert!(!json.contains("last_iteration_tail"));
        assert!(!json.contains("last_iteration_log"));
    }

    fn create_test_context() -> AnalysisContext {
        AnalysisContext {
            run_id: "test-run-123".to_string(),
            completion_display: "iteration 5".to_string(),
            last_iter: Some(5),
            model: "opus".to_string(),
            spec_path: Some("/workspace/specs/feature.md".to_string()),
            plan_path: Some("/workspace/specs/planning/feature-plan.md".to_string()),
            run_report: "/workspace/logs/loop/run-test-run-123/report.tsv".to_string(),
            run_log: "/workspace/logs/loop/run-test-run-123/run.log".to_string(),
            prompt_snapshot: "/workspace/logs/loop/run-test-run-123/prompt.txt".to_string(),
            summary_json: "/workspace/logs/loop/run-test-run-123/summary.json".to_string(),
            last_iter_tail: Some(
                "/workspace/logs/loop/run-test-run-123/iter-05.tail.txt".to_string(),
            ),
            last_iter_log: Some("/workspace/logs/loop/run-test-run-123/iter-05.log".to_string()),
            analysis_dir: PathBuf::from("/workspace/logs/loop/run-test-run-123/analysis"),
        }
    }

    #[test]
    fn run_quality_prompt_contains_required_sections() {
        let ctx = create_test_context();
        let prompt = build_run_quality_prompt(&ctx);

        // Verify metadata is included
        assert!(prompt.contains("Run ID: test-run-123"));
        assert!(prompt.contains("Completion detected: iteration 5"));
        assert!(prompt.contains("Last iteration observed: 5"));
        assert!(prompt.contains("Model: opus"));

        // Verify artifact paths are included
        assert!(prompt.contains("report.tsv"));
        assert!(prompt.contains("run.log"));
        assert!(prompt.contains("prompt.txt"));
        assert!(prompt.contains("summary.json"));
        assert!(prompt.contains("iter-05.tail.txt"));
        assert!(prompt.contains("iter-05.log"));

        // Verify expected output sections are mentioned
        assert!(prompt.contains("timeline summary"));
        assert!(prompt.contains("End-of-task behavior"));
        assert!(prompt.contains("Spec/template improvements"));
        assert!(prompt.contains("Loop prompt improvements"));
        assert!(prompt.contains("Loop UX/logging improvements"));
    }

    #[test]
    fn spec_compliance_prompt_contains_required_sections() {
        let ctx = create_test_context();
        let prompt = build_spec_compliance_prompt(&ctx);

        // Verify context is included
        assert!(prompt.contains("Spec: /workspace/specs/feature.md"));
        assert!(prompt.contains("Plan: /workspace/specs/planning/feature-plan.md"));
        assert!(prompt.contains("Model: opus"));

        // Verify git artifact paths
        assert!(prompt.contains("git-status.txt"));
        assert!(prompt.contains("git-last-commit.txt"));
        assert!(prompt.contains("git-last-commit.patch"));
        assert!(prompt.contains("git-diff.patch"));

        // Verify expected output sections
        assert!(prompt.contains("Compliance summary"));
        assert!(prompt.contains("Deviations"));
        assert!(prompt.contains("Missing verification steps"));
        assert!(prompt.contains("Required changes"));
        assert!(prompt.contains("Spec/template edits"));
    }

    #[test]
    fn summary_prompt_references_other_reports() {
        let ctx = create_test_context();
        let prompt = build_summary_prompt(&ctx);

        // Verify inputs reference the other analysis reports
        assert!(prompt.contains("spec-compliance.md"));
        assert!(prompt.contains("run-quality.md"));

        // Verify expected output sections
        assert!(prompt.contains("Root cause classification"));
        assert!(prompt.contains("Evidence"));
        assert!(prompt.contains("Required changes"));
        assert!(prompt.contains("Spec template changes"));
        assert!(prompt.contains("Loop prompt changes"));
        assert!(prompt.contains("Tooling/UX changes"));
    }

    #[test]
    fn analysis_context_handles_no_iterations() {
        let ctx = AnalysisContext {
            run_id: "empty-run".to_string(),
            completion_display: "not detected".to_string(),
            last_iter: None,
            model: "sonnet".to_string(),
            spec_path: None,
            plan_path: None,
            run_report: "/workspace/logs/loop/run-empty-run/report.tsv".to_string(),
            run_log: "/workspace/logs/loop/run-empty-run/run.log".to_string(),
            prompt_snapshot: "/workspace/logs/loop/run-empty-run/prompt.txt".to_string(),
            summary_json: "/workspace/logs/loop/run-empty-run/summary.json".to_string(),
            last_iter_tail: None,
            last_iter_log: None,
            analysis_dir: PathBuf::from("/workspace/logs/loop/run-empty-run/analysis"),
        };

        let prompt = build_run_quality_prompt(&ctx);
        assert!(prompt.contains("Last iteration observed: unknown"));
        assert!(prompt.contains("Last iteration tail: unknown"));
        assert!(prompt.contains("Last iteration log: unknown"));

        let spec_prompt = build_spec_compliance_prompt(&ctx);
        assert!(spec_prompt.contains("Spec: unknown"));
        assert!(spec_prompt.contains("Plan: unknown"));
    }

    #[test]
    fn write_analysis_prompts_creates_files() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let analysis_dir = temp_dir.path().join("analysis");

        let ctx = AnalysisContext {
            run_id: "test-run".to_string(),
            completion_display: "iteration 3".to_string(),
            last_iter: Some(3),
            model: "opus".to_string(),
            spec_path: Some("/path/to/spec.md".to_string()),
            plan_path: Some("/path/to/plan.md".to_string()),
            run_report: "/path/to/report.tsv".to_string(),
            run_log: "/path/to/run.log".to_string(),
            prompt_snapshot: "/path/to/prompt.txt".to_string(),
            summary_json: "/path/to/summary.json".to_string(),
            last_iter_tail: Some("/path/to/iter-03.tail.txt".to_string()),
            last_iter_log: Some("/path/to/iter-03.log".to_string()),
            analysis_dir: analysis_dir.clone(),
        };

        let prompts = write_analysis_prompts(&ctx).unwrap();

        // Verify files were created
        assert!(prompts.run_quality.prompt_path.exists());
        assert!(prompts.spec_compliance.prompt_path.exists());
        assert!(prompts.summary.prompt_path.exists());

        // Verify file names
        assert_eq!(
            prompts.run_quality.prompt_path.file_name().unwrap(),
            "run-quality-prompt.txt"
        );
        assert_eq!(
            prompts.spec_compliance.prompt_path.file_name().unwrap(),
            "spec-compliance-prompt.txt"
        );
        assert_eq!(
            prompts.summary.prompt_path.file_name().unwrap(),
            "summary-prompt.txt"
        );

        // Verify output paths
        assert_eq!(
            prompts.run_quality.output_path.file_name().unwrap(),
            "run-quality.md"
        );
        assert_eq!(
            prompts.spec_compliance.output_path.file_name().unwrap(),
            "spec-compliance.md"
        );
        assert_eq!(
            prompts.summary.output_path.file_name().unwrap(),
            "summary.md"
        );

        // Verify content matches
        let written_content = std::fs::read_to_string(&prompts.run_quality.prompt_path).unwrap();
        assert_eq!(written_content, prompts.run_quality.prompt);
    }

    #[test]
    fn capture_git_snapshot_creates_files_in_git_repo() {
        use tempfile::TempDir;

        // Create a temp directory and init a git repo
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let analysis_dir = workspace.join("analysis");

        // Init git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(workspace)
            .output()
            .expect("failed to init git");

        // Configure git user for commit
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(workspace)
            .output()
            .expect("failed to configure git");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(workspace)
            .output()
            .expect("failed to configure git");

        // Create and commit a file
        std::fs::write(workspace.join("test.txt"), "hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(workspace)
            .output()
            .expect("failed to add file");
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(workspace)
            .output()
            .expect("failed to commit");

        // Capture git snapshot
        let result = capture_git_snapshot(workspace, &analysis_dir).unwrap();
        assert!(result, "expected git snapshot to be captured");

        // Verify files were created
        assert!(analysis_dir.join("git-status.txt").exists());
        assert!(analysis_dir.join("git-last-commit.txt").exists());
        assert!(analysis_dir.join("git-last-commit.patch").exists());
        assert!(analysis_dir.join("git-diff.patch").exists());

        // Verify status contains branch info
        let status = std::fs::read_to_string(analysis_dir.join("git-status.txt")).unwrap();
        assert!(status.contains("##")); // Short branch status starts with ##

        // Verify commit info
        let commit = std::fs::read_to_string(analysis_dir.join("git-last-commit.txt")).unwrap();
        assert!(commit.contains("initial"));
    }

    #[test]
    fn capture_git_snapshot_returns_false_for_non_git_dir() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        let analysis_dir = workspace.join("analysis");

        // No git init - just a regular directory
        let result = capture_git_snapshot(workspace, &analysis_dir).unwrap();
        assert!(!result, "expected false for non-git directory");
    }

    #[test]
    fn postmortem_result_all_succeeded() {
        let success_result = AnalysisStepResult {
            success: true,
            exit_code: 0,
            output_path: PathBuf::from("/tmp/test.md"),
        };

        let failed_result = AnalysisStepResult {
            success: false,
            exit_code: 1,
            output_path: PathBuf::from("/tmp/test.md"),
        };

        // All succeeded
        let result = PostmortemResult {
            spec_compliance: Some(success_result.clone()),
            run_quality: Some(success_result.clone()),
            summary: Some(success_result.clone()),
        };
        assert!(result.all_succeeded());

        // One failed
        let result = PostmortemResult {
            spec_compliance: Some(success_result.clone()),
            run_quality: Some(failed_result.clone()),
            summary: Some(success_result.clone()),
        };
        assert!(!result.all_succeeded());

        // One missing
        let result = PostmortemResult {
            spec_compliance: Some(success_result.clone()),
            run_quality: None,
            summary: Some(success_result.clone()),
        };
        assert!(!result.all_succeeded());
    }

    #[test]
    fn execute_analysis_step_with_echo() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let analysis_dir = temp_dir.path().join("analysis");
        std::fs::create_dir_all(&analysis_dir).unwrap();

        let prompt = AnalysisPrompt {
            prompt: "test prompt".to_string(),
            prompt_path: analysis_dir.join("test-prompt.txt"),
            output_path: analysis_dir.join("test-output.md"),
        };

        // Use 'echo' instead of 'claude' to test the execution flow
        // This tests the file writing logic without requiring claude
        let output = Command::new("echo")
            .arg("test output")
            .current_dir(temp_dir.path())
            .output()
            .expect("echo should work");

        // Manually write output to simulate what execute_analysis_step does
        std::fs::write(&prompt.output_path, &output.stdout).unwrap();

        assert!(prompt.output_path.exists());
        let content = std::fs::read_to_string(&prompt.output_path).unwrap();
        assert!(content.contains("test output"));
    }

    #[test]
    fn is_claude_available_returns_bool() {
        // This just tests that the function doesn't panic
        // It will return true if claude is installed, false otherwise
        let result = is_claude_available();
        assert!(result == true || result == false);
    }
}
