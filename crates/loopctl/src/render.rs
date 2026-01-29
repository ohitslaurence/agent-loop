//! Output rendering for loopctl CLI.
//!
//! Formats run and step information for terminal display.
//! See spec Section 7.2 for diagnostics output requirements.

use loop_core::types::{Run, RunStatus, Step, StepStatus};
use std::fmt::Write;

/// Print confirmation after creating a run.
pub fn print_run_created(run: &Run) {
    print!("{}", render_run_created(run));
}

/// Render run creation confirmation to string.
pub fn render_run_created(run: &Run) -> String {
    let mut out = String::new();
    writeln!(out, "Created run: {}", run.id).unwrap();
    writeln!(out, "  Name:   {}", run.name).unwrap();
    writeln!(out, "  Spec:   {}", run.spec_path).unwrap();
    if let Some(ref plan) = run.plan_path {
        writeln!(out, "  Plan:   {}", plan).unwrap();
    }
    writeln!(out, "  Status: {}", format_status(run.status)).unwrap();
    out
}

/// Print a list of runs in tabular format.
pub fn print_run_list(runs: &[Run]) {
    print!("{}", render_run_list(runs));
}

/// Render run list to string.
pub fn render_run_list(runs: &[Run]) -> String {
    let mut out = String::new();

    if runs.is_empty() {
        writeln!(out, "No runs found.").unwrap();
        return out;
    }

    // Header
    writeln!(
        out,
        "{:<36}  {:<20}  {:<10}  {:<20}",
        "ID", "NAME", "STATUS", "CREATED"
    )
    .unwrap();
    writeln!(out, "{}", "-".repeat(90)).unwrap();

    for run in runs {
        writeln!(
            out,
            "{:<36}  {:<20}  {:<10}  {:<20}",
            run.id.0,
            truncate(&run.name, 20),
            format_status(run.status),
            format_time(&run.created_at),
        )
        .unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "{} run(s)", runs.len()).unwrap();
    out
}

/// Print detailed information about a run and its steps.
pub fn print_run_details(run: &Run, steps: &[Step]) {
    print!("{}", render_run_details(run, steps));
}

/// Render detailed run information to string.
///
/// Per spec Section 7.2, this shows:
/// - Run state (id, name, status, workspace, spec, plan)
/// - Last step information
/// - Artifact paths (log dir, prompt.txt, summary.json, report.tsv)
pub fn render_run_details(run: &Run, steps: &[Step]) -> String {
    let mut out = String::new();

    writeln!(out, "Run: {}", run.id).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "  Name:           {}", run.name).unwrap();
    writeln!(out, "  Name Source:    {}", run.name_source.as_str()).unwrap();
    writeln!(out, "  Status:         {}", format_status(run.status)).unwrap();
    writeln!(out, "  Workspace:      {}", run.workspace_root).unwrap();
    writeln!(out, "  Spec:           {}", run.spec_path).unwrap();
    if let Some(ref plan) = run.plan_path {
        writeln!(out, "  Plan:           {}", plan).unwrap();
    }

    // Worktree info
    if let Some(ref wt) = run.worktree {
        writeln!(out).unwrap();
        writeln!(out, "  Worktree:").unwrap();
        writeln!(out, "    Base Branch:    {}", wt.base_branch).unwrap();
        writeln!(out, "    Run Branch:     {}", wt.run_branch).unwrap();
        if let Some(ref target) = wt.merge_target_branch {
            writeln!(out, "    Merge Target:   {}", target).unwrap();
            writeln!(out, "    Merge Strategy: {}", wt.merge_strategy.as_str()).unwrap();
        }
        writeln!(out, "    Path:           {}", wt.worktree_path).unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "  Created:        {}", format_time(&run.created_at)).unwrap();
    writeln!(out, "  Updated:        {}", format_time(&run.updated_at)).unwrap();

    // Steps
    if !steps.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "  Steps:").unwrap();
        writeln!(
            out,
            "    {:<36}  {:<14}  {:<12}  {:<6}  {:<8}",
            "ID", "PHASE", "STATUS", "ATTEMPT", "EXIT"
        )
        .unwrap();
        writeln!(out, "    {}", "-".repeat(80)).unwrap();

        for step in steps {
            let exit_code = step
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string());
            writeln!(
                out,
                "    {:<36}  {:<14}  {:<12}  {:<6}  {:<8}",
                step.id.0,
                step.phase.as_str(),
                format_step_status(step.status),
                step.attempt,
                exit_code,
            )
            .unwrap();
        }
    }

    // Artifact paths (spec Section 7.2)
    writeln!(out).unwrap();
    writeln!(out, "  Artifacts:").unwrap();
    let log_dir = format!("{}/logs/loop/run-{}", run.workspace_root, run.id);
    writeln!(out, "    Log Dir:      {}", log_dir).unwrap();
    writeln!(out, "    Prompt:       {}/prompt.txt", log_dir).unwrap();
    writeln!(out, "    Summary:      {}/summary.json", log_dir).unwrap();
    writeln!(out, "    Report:       {}/report.tsv", log_dir).unwrap();

    out
}

fn format_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "PENDING",
        RunStatus::Running => "RUNNING",
        RunStatus::Paused => "PAUSED",
        RunStatus::Completed => "COMPLETED",
        RunStatus::Failed => "FAILED",
        RunStatus::Canceled => "CANCELED",
    }
}

fn format_step_status(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Queued => "QUEUED",
        StepStatus::InProgress => "IN_PROGRESS",
        StepStatus::Succeeded => "SUCCEEDED",
        StepStatus::Failed => "FAILED",
        StepStatus::Retrying => "RETRYING",
        StepStatus::Canceled => "CANCELED",
    }
}

fn format_time(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use loop_core::types::{Id, MergeStrategy, RunNameSource, RunWorktree, StepPhase};

    fn make_test_run() -> Run {
        Run {
            id: Id::from_string("01HQRS12345678901234567890"),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Running,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_test_step(run_id: &Id) -> Step {
        Step {
            id: Id::from_string("01HQRS98765432109876543210"),
            run_id: run_id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::Succeeded,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: Some(Utc::now()),
            exit_code: Some(0),
            prompt_path: Some("/workspace/logs/loop/prompt.txt".to_string()),
            output_path: Some("/workspace/logs/loop/output.log".to_string()),
        }
    }

    // --- Spec Section 7.2: inspect shows run state, last step, and artifact paths ---

    #[test]
    fn inspect_shows_run_state() {
        let run = make_test_run();
        let output = render_run_details(&run, &[]);

        // Verify run state is shown
        assert!(output.contains("Run: 01HQRS12345678901234567890"));
        assert!(output.contains("Name:           test-run"));
        assert!(output.contains("Name Source:    spec_slug"));
        assert!(output.contains("Status:         RUNNING"));
        assert!(output.contains("Workspace:      /workspace"));
        assert!(output.contains("Spec:           /workspace/spec.md"));
        assert!(output.contains("Plan:           /workspace/plan.md"));
    }

    #[test]
    fn inspect_shows_last_step() {
        let run = make_test_run();
        let step = make_test_step(&run.id);
        let output = render_run_details(&run, &[step]);

        // Verify step info is shown
        assert!(output.contains("Steps:"));
        assert!(output.contains("01HQRS98765432109876543210"));
        assert!(output.contains("implementation"));
        assert!(output.contains("SUCCEEDED"));
        assert!(output.contains("1")); // attempt
        assert!(output.contains("0")); // exit code
    }

    #[test]
    fn inspect_shows_artifact_paths() {
        let run = make_test_run();
        let output = render_run_details(&run, &[]);

        // Verify artifact paths per spec Section 7.2
        assert!(output.contains("Artifacts:"));
        assert!(
            output.contains("Log Dir:      /workspace/logs/loop/run-01HQRS12345678901234567890")
        );
        assert!(output.contains(
            "Prompt:       /workspace/logs/loop/run-01HQRS12345678901234567890/prompt.txt"
        ));
        assert!(output.contains(
            "Summary:      /workspace/logs/loop/run-01HQRS12345678901234567890/summary.json"
        ));
        assert!(output.contains(
            "Report:       /workspace/logs/loop/run-01HQRS12345678901234567890/report.tsv"
        ));
    }

    #[test]
    fn inspect_shows_worktree_info_when_present() {
        let mut run = make_test_run();
        run.worktree = Some(RunWorktree {
            base_branch: "main".to_string(),
            run_branch: "run/test-run".to_string(),
            merge_target_branch: Some("agent/test".to_string()),
            merge_strategy: MergeStrategy::Squash,
            worktree_path: "../workspace.run-test-run".to_string(),
        });

        let output = render_run_details(&run, &[]);

        // Verify worktree info is shown
        assert!(output.contains("Worktree:"));
        assert!(output.contains("Base Branch:    main"));
        assert!(output.contains("Run Branch:     run/test-run"));
        assert!(output.contains("Merge Target:   agent/test"));
        assert!(output.contains("Merge Strategy: squash"));
        assert!(output.contains("Path:           ../workspace.run-test-run"));
    }

    // --- Status formatting tests ---

    #[test]
    fn run_status_formats_correctly() {
        assert_eq!(format_status(RunStatus::Pending), "PENDING");
        assert_eq!(format_status(RunStatus::Running), "RUNNING");
        assert_eq!(format_status(RunStatus::Paused), "PAUSED");
        assert_eq!(format_status(RunStatus::Completed), "COMPLETED");
        assert_eq!(format_status(RunStatus::Failed), "FAILED");
        assert_eq!(format_status(RunStatus::Canceled), "CANCELED");
    }

    #[test]
    fn step_status_formats_correctly() {
        assert_eq!(format_step_status(StepStatus::Queued), "QUEUED");
        assert_eq!(format_step_status(StepStatus::InProgress), "IN_PROGRESS");
        assert_eq!(format_step_status(StepStatus::Succeeded), "SUCCEEDED");
        assert_eq!(format_step_status(StepStatus::Failed), "FAILED");
        assert_eq!(format_step_status(StepStatus::Retrying), "RETRYING");
        assert_eq!(format_step_status(StepStatus::Canceled), "CANCELED");
    }

    // --- Run list formatting tests ---

    #[test]
    fn run_list_shows_empty_message() {
        let output = render_run_list(&[]);
        assert!(output.contains("No runs found."));
    }

    #[test]
    fn run_list_shows_header_and_runs() {
        let run = make_test_run();
        let output = render_run_list(&[run]);

        assert!(output.contains("ID"));
        assert!(output.contains("NAME"));
        assert!(output.contains("STATUS"));
        assert!(output.contains("CREATED"));
        assert!(output.contains("01HQRS12345678901234567890"));
        assert!(output.contains("test-run"));
        assert!(output.contains("RUNNING"));
        assert!(output.contains("1 run(s)"));
    }

    #[test]
    fn truncate_long_name() {
        let result = truncate("this-is-a-very-long-run-name-that-exceeds-limit", 20);
        assert_eq!(result, "this-is-a-very-lo...");
        assert_eq!(result.len(), 20);
    }

    #[test]
    fn truncate_short_name() {
        let result = truncate("short-name", 20);
        assert_eq!(result, "short-name");
    }

    // --- Time formatting test ---

    #[test]
    fn format_time_uses_iso_style() {
        use chrono::TimeZone;
        let dt = Utc.with_ymd_and_hms(2026, 1, 29, 14, 30, 0).unwrap();
        let formatted = format_time(&dt);
        assert_eq!(formatted, "2026-01-29 14:30:00");
    }

    // --- Run created formatting tests ---

    #[test]
    fn run_created_shows_basic_info() {
        let run = make_test_run();
        let output = render_run_created(&run);

        assert!(output.contains("Created run: 01HQRS12345678901234567890"));
        assert!(output.contains("Name:   test-run"));
        assert!(output.contains("Spec:   /workspace/spec.md"));
        assert!(output.contains("Plan:   /workspace/plan.md"));
        assert!(output.contains("Status: RUNNING"));
    }

    #[test]
    fn run_created_hides_plan_when_absent() {
        let mut run = make_test_run();
        run.plan_path = None;
        let output = render_run_created(&run);

        assert!(!output.contains("Plan:"));
    }

    // --- Multiple steps test ---

    #[test]
    fn inspect_shows_multiple_steps() {
        let run = make_test_run();
        let step1 = Step {
            id: Id::from_string("step-1"),
            run_id: run.id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::Succeeded,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: Some(Utc::now()),
            exit_code: Some(0),
            prompt_path: None,
            output_path: None,
        };
        let step2 = Step {
            id: Id::from_string("step-2"),
            run_id: run.id.clone(),
            phase: StepPhase::Review,
            status: StepStatus::InProgress,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        };

        let output = render_run_details(&run, &[step1, step2]);

        assert!(output.contains("step-1"));
        assert!(output.contains("step-2"));
        assert!(output.contains("implementation"));
        assert!(output.contains("review"));
        assert!(output.contains("SUCCEEDED"));
        assert!(output.contains("IN_PROGRESS"));
    }

    #[test]
    fn inspect_shows_dash_for_missing_exit_code() {
        let run = make_test_run();
        let step = Step {
            id: Id::from_string("step-in-progress"),
            run_id: run.id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::InProgress,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        };

        let output = render_run_details(&run, &[step]);

        // Check that "-" is shown for missing exit code
        // The line should contain the step ID followed by phase, status, attempt, and "-"
        assert!(output.contains("step-in-progress"));
        // Verify the dash is in the exit code column (step line contains "-" for exit)
        let lines: Vec<&str> = output.lines().collect();
        let step_line = lines
            .iter()
            .find(|l| l.contains("step-in-progress"))
            .unwrap();
        // The exit code column shows "-" (padded with spaces due to column width)
        assert!(step_line.trim_end().ends_with("-"));
    }
}
