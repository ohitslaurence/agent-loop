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
    /// Worktree removed after run completion (worktrunk-integration.md Section 4.3).
    WorktreeRemoved,
    /// Postmortem analysis started (postmortem-analysis.md Section 4).
    PostmortemStart,
    /// Postmortem analysis ended (postmortem-analysis.md Section 4).
    PostmortemEnd,
    /// Skill body was truncated (open-skills-orchestration.md Section 4.3).
    SkillsTruncated,
    /// Skills discovered during run execution (open-skills-orchestration.md Section 4.3).
    SkillsDiscovered,
    /// Skills selected for a task (open-skills-orchestration.md Section 4.3).
    SkillsSelected,
    /// Skill load failed (open-skills-orchestration.md Section 4.3).
    SkillsLoadFailed,
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
            Self::WorktreeRemoved => "WORKTREE_REMOVED",
            Self::PostmortemStart => "POSTMORTEM_START",
            Self::PostmortemEnd => "POSTMORTEM_END",
            Self::SkillsTruncated => "SKILLS_TRUNCATED",
            Self::SkillsDiscovered => "SKILLS_DISCOVERED",
            Self::SkillsSelected => "SKILLS_SELECTED",
            Self::SkillsLoadFailed => "SKILLS_LOAD_FAILED",
        }
    }
}

/// Payload for `RUN_CREATED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreatedPayload {
    pub run_id: Id,
    pub name: String,
    pub name_source: RunNameSource,
    pub spec_path: String,
    pub plan_path: Option<String>,
}

/// Payload for `RUN_STARTED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStartedPayload {
    pub run_id: Id,
    pub worker_id: String,
}

/// Payload for `STEP_STARTED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStartedPayload {
    pub step_id: Id,
    pub phase: String,
    pub attempt: u32,
}

/// Payload for `STEP_FINISHED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepFinishedPayload {
    pub step_id: Id,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub output_path: String,
}

/// Payload for `WATCHDOG_REWRITE` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogRewritePayload {
    pub step_id: Id,
    pub signal: WatchdogSignal,
    pub prompt_before: String,
    pub prompt_after: String,
}

/// Payload for `RUN_COMPLETED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCompletedPayload {
    pub run_id: Id,
    /// Completion mode that triggered success.
    pub mode: String,
}

/// Payload for `RUN_FAILED` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFailedPayload {
    pub run_id: Id,
    pub reason: String,
}

/// Payload for `WORKTREE_PROVIDER_SELECTED` event.
///
/// See worktrunk-integration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeProviderSelectedPayload {
    pub run_id: Id,
    pub provider: WorktreeProvider,
}

/// Payload for `WORKTREE_CREATED` event.
///
/// See worktrunk-integration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreatedPayload {
    pub run_id: Id,
    pub provider: WorktreeProvider,
    pub worktree_path: String,
    pub run_branch: String,
}

/// Payload for `WORKTREE_REMOVED` event.
///
/// See worktrunk-integration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRemovedPayload {
    pub run_id: Id,
    pub provider: WorktreeProvider,
    pub worktree_path: String,
}

/// Payload for `POSTMORTEM_START` event.
///
/// See postmortem-analysis.md Section 4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostmortemStartPayload {
    pub run_id: Id,
    /// Reason for triggering postmortem (e.g., "`run_completed`", "`run_failed`", "manual").
    pub reason: String,
}

/// Payload for `POSTMORTEM_END` event.
///
/// See postmortem-analysis.md Section 4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostmortemEndPayload {
    pub run_id: Id,
    /// Status of postmortem analysis: "ok" or "failed".
    pub status: String,
}

/// Payload for `SKILLS_TRUNCATED` event.
///
/// See open-skills-orchestration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsTruncatedPayload {
    pub run_id: Id,
    /// The skill name that was truncated.
    pub name: String,
    /// The maximum character limit that was applied.
    pub max_chars: usize,
}

/// Payload for `SKILLS_DISCOVERED` event.
///
/// See open-skills-orchestration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsDiscoveredPayload {
    pub run_id: Id,
    /// Number of skills discovered.
    pub count: usize,
    /// Locations where skills were found (e.g., ["project", "global"]).
    pub locations: Vec<String>,
    /// Names of discovered skills.
    pub names: Vec<String>,
}

/// Payload for `SKILLS_LOAD_FAILED` event.
///
/// See open-skills-orchestration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsLoadFailedPayload {
    pub run_id: Id,
    /// Skill name that failed to load.
    pub name: String,
    /// Error message describing the failure.
    pub error: String,
}

/// Selected skill with reason for selection.
///
/// See open-skills-orchestration.md Section 3.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedSkillPayload {
    /// Skill name.
    pub name: String,
    /// Reason for selection (e.g., "hint: @skill-name" or "keyword: pdf").
    pub reason: String,
}

/// Payload for `SKILLS_SELECTED` event.
///
/// See open-skills-orchestration.md Section 4.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSelectedPayload {
    pub run_id: Id,
    /// Step kind (implementation or review).
    pub step_kind: String,
    /// Task label from the plan.
    pub task_label: String,
    /// Selected skills with reasons.
    pub skills: Vec<SelectedSkillPayload>,
    /// Selection strategy used (hint, match, or none).
    pub strategy: String,
    /// Errors encountered during selection (e.g., hinted skill not found).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
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
    WorktreeRemoved(WorktreeRemovedPayload),
    PostmortemStart(PostmortemStartPayload),
    PostmortemEnd(PostmortemEndPayload),
    SkillsTruncated(SkillsTruncatedPayload),
    SkillsDiscovered(SkillsDiscoveredPayload),
    SkillsSelected(SkillsSelectedPayload),
    SkillsLoadFailed(SkillsLoadFailedPayload),
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
            Self::WorktreeRemoved(_) => EventType::WorktreeRemoved,
            Self::PostmortemStart(_) => EventType::PostmortemStart,
            Self::PostmortemEnd(_) => EventType::PostmortemEnd,
            Self::SkillsTruncated(_) => EventType::SkillsTruncated,
            Self::SkillsDiscovered(_) => EventType::SkillsDiscovered,
            Self::SkillsSelected(_) => EventType::SkillsSelected,
            Self::SkillsLoadFailed(_) => EventType::SkillsLoadFailed,
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

    /// Verify WORKTREE_PROVIDER_SELECTED payload matches Section 4.3:
    /// {run_id, provider}
    #[test]
    fn worktree_provider_selected_payload_serializes() {
        let payload = WorktreeProviderSelectedPayload {
            run_id: Id::from_string("run-123"),
            provider: WorktreeProvider::Worktrunk,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-123");
        assert_eq!(parsed["provider"], "worktrunk");

        // Verify round-trip
        let deserialized: WorktreeProviderSelectedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-123");
        assert_eq!(deserialized.provider, WorktreeProvider::Worktrunk);
    }

    /// Verify WORKTREE_CREATED payload matches Section 4.3:
    /// {run_id, provider, worktree_path, run_branch}
    #[test]
    fn worktree_created_payload_serializes() {
        let payload = WorktreeCreatedPayload {
            run_id: Id::from_string("run-456"),
            provider: WorktreeProvider::Git,
            worktree_path: "/worktrees/my-branch".to_string(),
            run_branch: "loop/run-456".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-456");
        assert_eq!(parsed["provider"], "git");
        assert_eq!(parsed["worktree_path"], "/worktrees/my-branch");
        assert_eq!(parsed["run_branch"], "loop/run-456");

        // Verify round-trip
        let deserialized: WorktreeCreatedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-456");
        assert_eq!(deserialized.provider, WorktreeProvider::Git);
        assert_eq!(deserialized.worktree_path, "/worktrees/my-branch");
        assert_eq!(deserialized.run_branch, "loop/run-456");
    }

    /// Verify WORKTREE_REMOVED payload matches Section 4.3:
    /// {run_id, provider, worktree_path}
    #[test]
    fn worktree_removed_payload_serializes() {
        let payload = WorktreeRemovedPayload {
            run_id: Id::from_string("run-789"),
            provider: WorktreeProvider::Auto,
            worktree_path: "/worktrees/cleanup-test".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-789");
        assert_eq!(parsed["provider"], "auto");
        assert_eq!(parsed["worktree_path"], "/worktrees/cleanup-test");

        // Verify round-trip
        let deserialized: WorktreeRemovedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-789");
        assert_eq!(deserialized.provider, WorktreeProvider::Auto);
        assert_eq!(deserialized.worktree_path, "/worktrees/cleanup-test");
    }

    /// Verify EventPayload wrapper correctly identifies event types for worktree events.
    #[test]
    fn worktree_event_payloads_via_union() {
        let selected = EventPayload::WorktreeProviderSelected(WorktreeProviderSelectedPayload {
            run_id: Id::from_string("r1"),
            provider: WorktreeProvider::Worktrunk,
        });
        assert_eq!(selected.event_type(), EventType::WorktreeProviderSelected);

        let created = EventPayload::WorktreeCreated(WorktreeCreatedPayload {
            run_id: Id::from_string("r2"),
            provider: WorktreeProvider::Git,
            worktree_path: "/wt".to_string(),
            run_branch: "branch".to_string(),
        });
        assert_eq!(created.event_type(), EventType::WorktreeCreated);

        let removed = EventPayload::WorktreeRemoved(WorktreeRemovedPayload {
            run_id: Id::from_string("r3"),
            provider: WorktreeProvider::Auto,
            worktree_path: "/wt".to_string(),
        });
        assert_eq!(removed.event_type(), EventType::WorktreeRemoved);
    }

    /// Verify POSTMORTEM_START payload matches Section 4:
    /// {run_id, reason}
    #[test]
    fn postmortem_start_payload_serializes() {
        let payload = PostmortemStartPayload {
            run_id: Id::from_string("run-pm-1"),
            reason: "run_completed".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-pm-1");
        assert_eq!(parsed["reason"], "run_completed");

        // Verify round-trip
        let deserialized: PostmortemStartPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-pm-1");
        assert_eq!(deserialized.reason, "run_completed");
    }

    /// Verify POSTMORTEM_END payload matches Section 4:
    /// {run_id, status} where status is "ok" or "failed"
    #[test]
    fn postmortem_end_payload_serializes() {
        let payload = PostmortemEndPayload {
            run_id: Id::from_string("run-pm-2"),
            status: "ok".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-pm-2");
        assert_eq!(parsed["status"], "ok");

        // Verify round-trip
        let deserialized: PostmortemEndPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-pm-2");
        assert_eq!(deserialized.status, "ok");

        // Test failed status
        let failed_payload = PostmortemEndPayload {
            run_id: Id::from_string("run-pm-3"),
            status: "failed".to_string(),
        };
        let failed_json = serde_json::to_string(&failed_payload).unwrap();
        assert!(failed_json.contains("\"status\":\"failed\""));
    }

    /// Verify EventPayload wrapper correctly identifies event types for postmortem events.
    #[test]
    fn postmortem_event_payloads_via_union() {
        let start = EventPayload::PostmortemStart(PostmortemStartPayload {
            run_id: Id::from_string("pm1"),
            reason: "manual".to_string(),
        });
        assert_eq!(start.event_type(), EventType::PostmortemStart);

        let end = EventPayload::PostmortemEnd(PostmortemEndPayload {
            run_id: Id::from_string("pm2"),
            status: "ok".to_string(),
        });
        assert_eq!(end.event_type(), EventType::PostmortemEnd);
    }

    /// Verify SKILLS_TRUNCATED payload matches Section 4.3:
    /// {run_id, name, max_chars}
    #[test]
    fn skills_truncated_payload_serializes() {
        let payload = SkillsTruncatedPayload {
            run_id: Id::from_string("run-sk-1"),
            name: "pdf-processing".to_string(),
            max_chars: 20000,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-sk-1");
        assert_eq!(parsed["name"], "pdf-processing");
        assert_eq!(parsed["max_chars"], 20000);

        // Verify round-trip
        let deserialized: SkillsTruncatedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-sk-1");
        assert_eq!(deserialized.name, "pdf-processing");
        assert_eq!(deserialized.max_chars, 20000);
    }

    /// Verify EventPayload wrapper correctly identifies event type for skills truncated.
    #[test]
    fn skills_truncated_event_payload_via_union() {
        let truncated = EventPayload::SkillsTruncated(SkillsTruncatedPayload {
            run_id: Id::from_string("sk1"),
            name: "test-skill".to_string(),
            max_chars: 10000,
        });
        assert_eq!(truncated.event_type(), EventType::SkillsTruncated);
    }

    /// Verify SKILLS_DISCOVERED payload matches Section 4.3:
    /// {run_id, count, locations, names}
    #[test]
    fn skills_discovered_payload_serializes() {
        let payload = SkillsDiscoveredPayload {
            run_id: Id::from_string("run-sd-1"),
            count: 3,
            locations: vec!["project".to_string(), "global".to_string()],
            names: vec![
                "pdf-processing".to_string(),
                "code-review".to_string(),
                "testing".to_string(),
            ],
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-sd-1");
        assert_eq!(parsed["count"], 3);
        assert_eq!(parsed["locations"][0], "project");
        assert_eq!(parsed["locations"][1], "global");
        assert_eq!(parsed["names"][0], "pdf-processing");

        // Verify round-trip
        let deserialized: SkillsDiscoveredPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-sd-1");
        assert_eq!(deserialized.count, 3);
        assert_eq!(deserialized.locations.len(), 2);
        assert_eq!(deserialized.names.len(), 3);
    }

    /// Verify SKILLS_LOAD_FAILED payload matches Section 4.3:
    /// {run_id, name, error}
    #[test]
    fn skills_load_failed_payload_serializes() {
        let payload = SkillsLoadFailedPayload {
            run_id: Id::from_string("run-slf-1"),
            name: "pdf-processing".to_string(),
            error: "failed to read SKILL.md".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-slf-1");
        assert_eq!(parsed["name"], "pdf-processing");
        assert_eq!(parsed["error"], "failed to read SKILL.md");

        // Verify round-trip
        let deserialized: SkillsLoadFailedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-slf-1");
        assert_eq!(deserialized.name, "pdf-processing");
        assert_eq!(deserialized.error, "failed to read SKILL.md");
    }

    /// Verify SKILLS_SELECTED payload matches Section 4.3 and Section 3.1:
    /// {run_id, step_kind, task_label, skills, strategy, errors}
    #[test]
    fn skills_selected_payload_serializes() {
        let payload = SkillsSelectedPayload {
            run_id: Id::from_string("run-ss-1"),
            step_kind: "implementation".to_string(),
            task_label: "Implement PDF export".to_string(),
            skills: vec![SelectedSkillPayload {
                name: "pdf-processing".to_string(),
                reason: "hint: @pdf-processing".to_string(),
            }],
            strategy: "hint".to_string(),
            errors: Vec::new(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["run_id"], "run-ss-1");
        assert_eq!(parsed["step_kind"], "implementation");
        assert_eq!(parsed["task_label"], "Implement PDF export");
        assert_eq!(parsed["skills"][0]["name"], "pdf-processing");
        assert_eq!(parsed["skills"][0]["reason"], "hint: @pdf-processing");
        assert_eq!(parsed["strategy"], "hint");
        // errors should be omitted when empty
        assert!(parsed.get("errors").is_none());

        // Verify round-trip
        let deserialized: SkillsSelectedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.run_id.0.as_str(), "run-ss-1");
        assert_eq!(deserialized.step_kind, "implementation");
        assert_eq!(deserialized.skills.len(), 1);
    }

    /// Verify SKILLS_SELECTED with errors is serialized correctly.
    #[test]
    fn skills_selected_payload_with_errors() {
        let payload = SkillsSelectedPayload {
            run_id: Id::from_string("run-ss-2"),
            step_kind: "review".to_string(),
            task_label: "Review @missing-skill".to_string(),
            skills: Vec::new(),
            strategy: "none".to_string(),
            errors: vec!["hinted skill not found: @missing-skill".to_string()],
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["errors"][0],
            "hinted skill not found: @missing-skill"
        );
    }

    /// Verify EventPayload wrapper correctly identifies event types for skills events.
    #[test]
    fn skills_event_payloads_via_union() {
        let discovered = EventPayload::SkillsDiscovered(SkillsDiscoveredPayload {
            run_id: Id::from_string("sd1"),
            count: 2,
            locations: vec!["project".to_string()],
            names: vec!["skill1".to_string(), "skill2".to_string()],
        });
        assert_eq!(discovered.event_type(), EventType::SkillsDiscovered);

        let selected = EventPayload::SkillsSelected(SkillsSelectedPayload {
            run_id: Id::from_string("ss1"),
            step_kind: "implementation".to_string(),
            task_label: "Test task".to_string(),
            skills: Vec::new(),
            strategy: "none".to_string(),
            errors: Vec::new(),
        });
        assert_eq!(selected.event_type(), EventType::SkillsSelected);

        let load_failed = EventPayload::SkillsLoadFailed(SkillsLoadFailedPayload {
            run_id: Id::from_string("slf1"),
            name: "bad-skill".to_string(),
            error: "invalid yaml".to_string(),
        });
        assert_eq!(load_failed.event_type(), EventType::SkillsLoadFailed);
    }
}
