//! Event types for the audit log.
//!
//! Event names and payloads match Section 4.3 of the spec.

use crate::types::{Id, RunNameSource, WatchdogSignal, WorktreeProvider};
use serde::{Deserialize, Serialize};

/// Event type names (Section 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    RunCreated,
    RunStarted,
    StepStarted,
    StepFinished,
    WatchdogRewrite,
    RunCompleted,
    RunFailed,
    /// Worktree provider resolved for a run (worktrunk-integration.md Section 4.3).
    WorktreeProviderSelected,
    /// Worktree created for a run (worktrunk-integration.md Section 4.3).
    WorktreeCreated,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunCreated => "RUN_CREATED",
            Self::RunStarted => "RUN_STARTED",
            Self::StepStarted => "STEP_STARTED",
            Self::StepFinished => "STEP_FINISHED",
            Self::WatchdogRewrite => "WATCHDOG_REWRITE",
            Self::RunCompleted => "RUN_COMPLETED",
            Self::RunFailed => "RUN_FAILED",
            Self::WorktreeProviderSelected => "WORKTREE_PROVIDER_SELECTED",
            Self::WorktreeCreated => "WORKTREE_CREATED",
        }
    }
}

/// Payload for RUN_CREATED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreatedPayload {
    pub run_id: Id,
    pub name: String,
    pub name_source: RunNameSource,
    pub spec_path: String,
    pub plan_path: Option<String>,
}

/// Payload for RUN_STARTED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStartedPayload {
    pub run_id: Id,
    pub worker_id: String,
}

/// Payload for STEP_STARTED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStartedPayload {
    pub step_id: Id,
    pub phase: String,
    pub attempt: u32,
}

/// Payload for STEP_FINISHED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFinishedPayload {
    pub step_id: Id,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub output_path: String,
}

/// Payload for WATCHDOG_REWRITE event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogRewritePayload {
    pub step_id: Id,
    pub signal: WatchdogSignal,
    pub prompt_before: String,
    pub prompt_after: String,
}

/// Payload for RUN_COMPLETED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCompletedPayload {
    pub run_id: Id,
    /// Completion mode that triggered success.
    pub mode: String,
}

/// Payload for RUN_FAILED event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFailedPayload {
    pub run_id: Id,
    pub reason: String,
}

/// Payload for WORKTREE_PROVIDER_SELECTED event.
///
/// See worktrunk-integration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeProviderSelectedPayload {
    pub run_id: Id,
    pub provider: WorktreeProvider,
}

/// Payload for WORKTREE_CREATED event.
///
/// See worktrunk-integration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreatedPayload {
    pub run_id: Id,
    pub provider: WorktreeProvider,
    pub worktree_path: String,
    pub run_branch: String,
}

/// Union type for all event payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventPayload {
    RunCreated(RunCreatedPayload),
    RunStarted(RunStartedPayload),
    StepStarted(StepStartedPayload),
    StepFinished(StepFinishedPayload),
    WatchdogRewrite(WatchdogRewritePayload),
    RunCompleted(RunCompletedPayload),
    RunFailed(RunFailedPayload),
    WorktreeProviderSelected(WorktreeProviderSelectedPayload),
    WorktreeCreated(WorktreeCreatedPayload),
}

impl EventPayload {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::RunCreated(_) => EventType::RunCreated,
            Self::RunStarted(_) => EventType::RunStarted,
            Self::StepStarted(_) => EventType::StepStarted,
            Self::StepFinished(_) => EventType::StepFinished,
            Self::WatchdogRewrite(_) => EventType::WatchdogRewrite,
            Self::RunCompleted(_) => EventType::RunCompleted,
            Self::RunFailed(_) => EventType::RunFailed,
            Self::WorktreeProviderSelected(_) => EventType::WorktreeProviderSelected,
            Self::WorktreeCreated(_) => EventType::WorktreeCreated,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&EventType::RunCreated).unwrap(),
            "\"RUN_CREATED\""
        );
        assert_eq!(
            serde_json::to_string(&EventType::StepFinished).unwrap(),
            "\"STEP_FINISHED\""
        );
    }

    #[test]
    fn run_created_payload_serializes() {
        let payload = RunCreatedPayload {
            run_id: Id::from_string("test-run"),
            name: "test-name".to_string(),
            name_source: RunNameSource::Haiku,
            spec_path: "/path/to/spec.md".to_string(),
            plan_path: Some("/path/to/plan.md".to_string()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("test-run"));
        assert!(json.contains("haiku"));
    }

    #[test]
    fn watchdog_rewrite_payload_matches_spec() {
        let payload = WatchdogRewritePayload {
            step_id: Id::from_string("01J2Z9"),
            signal: WatchdogSignal::NoProgress,
            prompt_before: "logs/loop/run-.../prompt.txt".to_string(),
            prompt_after: "logs/loop/run-.../prompt.rewrite.1.txt".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("no_progress"));
        assert!(json.contains("prompt_before"));
        assert!(json.contains("prompt_after"));
    }
}
