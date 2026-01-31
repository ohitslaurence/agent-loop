//! Core types for the orchestrator daemon.
//!
//! These types match the data model defined in the spec (Section 3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for runs, steps, events, and artifacts.
/// Uses `UUIDv7` for time-ordered lexicographic sorting.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub String);

impl Id {
    pub fn new() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Id {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// --- Enumerations (Section 3.2) ---

/// Run lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Paused => "PAUSED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Canceled => "CANCELED",
        }
    }
}

/// Step execution phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepPhase {
    Implementation,
    Review,
    Verification,
    Watchdog,
    Merge,
}

impl StepPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Implementation => "implementation",
            Self::Review => "review",
            Self::Verification => "verification",
            Self::Watchdog => "watchdog",
            Self::Merge => "merge",
        }
    }

    /// Short slug for artifact filenames (e.g., iter-01-impl.log).
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Implementation => "impl",
            Self::Review => "review",
            Self::Verification => "verify",
            Self::Watchdog => "watchdog",
            Self::Merge => "merge",
        }
    }
}

/// Step execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepStatus {
    Queued,
    InProgress,
    Succeeded,
    Failed,
    Retrying,
    Canceled,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::InProgress => "IN_PROGRESS",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Retrying => "RETRYING",
            Self::Canceled => "CANCELED",
        }
    }
}

/// Completion detection mode (matches bin/loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionMode {
    /// Output must be exactly `<promise>COMPLETE</promise>`.
    Exact,
    /// Last non-empty line must be `<promise>COMPLETE</promise>`.
    #[default]
    Trailing,
}

impl CompletionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Trailing => "trailing",
        }
    }
}

/// Watchdog signal types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchdogSignal {
    RepeatedTask,
    VerificationFailed,
    NoProgress,
    MalformedComplete,
}

impl WatchdogSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RepeatedTask => "repeated_task",
            Self::VerificationFailed => "verification_failed",
            Self::NoProgress => "no_progress",
            Self::MalformedComplete => "malformed_complete",
        }
    }
}

/// Artifact storage location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactLocation {
    Workspace,
    Global,
}

impl ArtifactLocation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Global => "global",
        }
    }
}

/// Source for run naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunNameSource {
    SpecSlug,
    #[default]
    Haiku,
}

impl RunNameSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SpecSlug => "spec_slug",
            Self::Haiku => "haiku",
        }
    }
}

/// Git merge strategy for run branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    None,
    Merge,
    #[default]
    Squash,
}

impl MergeStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Merge => "merge",
            Self::Squash => "squash",
        }
    }
}

/// Artifact mode for storage mirroring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMode {
    /// Store in workspace only.
    Workspace,
    /// Store in global only.
    Global,
    /// Store in both workspace and global.
    #[default]
    Mirror,
}

impl ArtifactMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Global => "global",
            Self::Mirror => "mirror",
        }
    }
}

/// Queue discipline policy for run scheduling.
///
/// See spec Section 3.2 (Enumerations) and Section 5.3 (Local scaling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePolicy {
    /// First-in, first-out: oldest pending run is claimed first.
    #[default]
    Fifo,
    /// Newest first: most recently created pending run is claimed first.
    NewestFirst,
}

impl QueuePolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fifo => "fifo",
            Self::NewestFirst => "newest_first",
        }
    }
}

/// Worktree provider selection.
///
/// See worktrunk-integration.md Section 3 (Data Model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeProvider {
    /// Auto-detect: use Worktrunk if available, else fallback to git.
    #[default]
    Auto,
    /// Use Worktrunk CLI (`wt`) for worktree lifecycle.
    Worktrunk,
    /// Use native git commands for worktree lifecycle.
    Git,
}

impl WorktreeProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Worktrunk => "worktrunk",
            Self::Git => "git",
        }
    }
}

// --- Core Types (Section 3.1) ---

/// Worktree and branch configuration for a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunWorktree {
    /// Base branch to create run branch from.
    pub base_branch: String,
    /// Branch name for this run (e.g., `run/<run_name_slug>`).
    pub run_branch: String,
    /// Target branch to merge into on completion (optional).
    pub merge_target_branch: Option<String>,
    /// Strategy for merging into target branch.
    pub merge_strategy: MergeStrategy,
    /// Path to the worktree directory.
    pub worktree_path: String,
    /// Worktree provider used for this run.
    pub provider: WorktreeProvider,
}

/// A single run of the agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: Id,
    /// Human-readable label (ASCII, max 64 chars).
    pub name: String,
    /// How the name was generated.
    pub name_source: RunNameSource,
    pub status: RunStatus,
    /// Absolute path to the workspace root (git root or cwd).
    pub workspace_root: String,
    /// Absolute path to the spec file.
    pub spec_path: String,
    /// Absolute path to the plan file (optional).
    pub plan_path: Option<String>,
    /// Worktree configuration.
    pub worktree: Option<RunWorktree>,
    /// Worktree cleanup status ("cleaned", "failed", "skipped") if cleanup attempted.
    pub worktree_cleanup_status: Option<String>,
    /// Timestamp when worktree cleanup completed successfully.
    pub worktree_cleaned_at: Option<DateTime<Utc>>,
    /// JSON-serialized config overrides.
    pub config_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single step (iteration) within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub id: Id,
    pub run_id: Id,
    pub phase: StepPhase,
    pub status: StepStatus,
    /// Attempt number (1-indexed, incremented on retry).
    pub attempt: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Exit code from Claude CLI (if completed).
    pub exit_code: Option<i32>,
    /// Path to the prompt file for this step.
    pub prompt_path: Option<String>,
    /// Path to the output log for this step.
    pub output_path: Option<String>,
}

/// An event in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Id,
    pub run_id: Id,
    /// Associated step (optional).
    pub step_id: Option<Id>,
    /// Event type name (e.g., `RUN_CREATED`, `STEP_FINISHED`).
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    /// JSON payload with event-specific data.
    pub payload_json: String,
}

/// An artifact file reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Id,
    pub run_id: Id,
    /// Kind of artifact (e.g., `prompt`, `output`, `summary`).
    pub kind: String,
    /// Storage location.
    pub location: ArtifactLocation,
    /// Absolute path to the artifact file.
    pub path: String,
    /// Checksum for integrity verification.
    pub checksum: Option<String>,
}

/// Watchdog decision after evaluating signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogDecision {
    pub signal: WatchdogSignal,
    /// Action taken (e.g., `rewrite`, `continue`, `fail`).
    pub action: String,
    /// Number of rewrites attempted for this run.
    pub rewrite_count: u32,
    /// Human-readable notes about the decision.
    pub notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_generates_unique_values() {
        let id1 = Id::new();
        let id2 = Id::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn run_status_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&RunStatus::Running).unwrap(),
            "\"RUNNING\""
        );
        assert_eq!(
            serde_json::to_string(&RunStatus::Pending).unwrap(),
            "\"PENDING\""
        );
    }

    #[test]
    fn step_phase_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&StepPhase::Implementation).unwrap(),
            "\"implementation\""
        );
    }

    #[test]
    fn completion_mode_default_is_trailing() {
        assert_eq!(CompletionMode::default(), CompletionMode::Trailing);
    }

    #[test]
    fn merge_strategy_default_is_squash() {
        assert_eq!(MergeStrategy::default(), MergeStrategy::Squash);
    }

    #[test]
    fn run_name_source_default_is_haiku() {
        assert_eq!(RunNameSource::default(), RunNameSource::Haiku);
    }

    #[test]
    fn worktree_provider_default_is_auto() {
        assert_eq!(WorktreeProvider::default(), WorktreeProvider::Auto);
    }

    #[test]
    fn worktree_provider_serializes_correctly() {
        assert_eq!(
            serde_json::to_string(&WorktreeProvider::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&WorktreeProvider::Worktrunk).unwrap(),
            "\"worktrunk\""
        );
        assert_eq!(
            serde_json::to_string(&WorktreeProvider::Git).unwrap(),
            "\"git\""
        );
    }

    #[test]
    fn worktree_provider_as_str() {
        assert_eq!(WorktreeProvider::Auto.as_str(), "auto");
        assert_eq!(WorktreeProvider::Worktrunk.as_str(), "worktrunk");
        assert_eq!(WorktreeProvider::Git.as_str(), "git");
    }
}
