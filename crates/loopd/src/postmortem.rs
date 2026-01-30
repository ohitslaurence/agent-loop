//! Postmortem and summary artifact generation.
//!
//! Implements run summaries and analysis reports.
//! See spec: specs/postmortem-analysis.md

use loop_core::artifacts::ArtifactError;
use loop_core::{write_and_mirror_artifact, Artifact, Config, Run, Step};
use serde::Serialize;
use std::path::Path;
use thiserror::Error;

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
    let completed_iteration =
        if exit_reason == ExitReason::CompletePlan || exit_reason == ExitReason::CompleteReviewer {
            Some(iterations_run)
        } else {
            None
        };

    // Calculate timing.
    let start_ms = run.created_at.timestamp_millis();
    let end_ms = chrono::Utc::now().timestamp_millis();
    let total_duration_ms = end_ms - start_ms;
    let avg_duration_ms = if iterations_run > 0 {
        total_duration_ms / iterations_run as i64
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
                .join(format!("{}.tail.txt", stem))
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
}
