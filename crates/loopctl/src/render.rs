//! Output rendering for loopctl CLI.
//!
//! Formats run and step information for terminal display.
//! See spec Section 7.2 for diagnostics output requirements.

use loop_core::types::{Run, RunStatus, Step, StepStatus};

/// Print confirmation after creating a run.
pub fn print_run_created(run: &Run) {
    println!("Created run: {}", run.id);
    println!("  Name:   {}", run.name);
    println!("  Spec:   {}", run.spec_path);
    if let Some(ref plan) = run.plan_path {
        println!("  Plan:   {}", plan);
    }
    println!("  Status: {}", format_status(run.status));
}

/// Print a list of runs in tabular format.
pub fn print_run_list(runs: &[Run]) {
    if runs.is_empty() {
        println!("No runs found.");
        return;
    }

    // Header
    println!(
        "{:<36}  {:<20}  {:<10}  {:<20}",
        "ID", "NAME", "STATUS", "CREATED"
    );
    println!("{}", "-".repeat(90));

    for run in runs {
        println!(
            "{:<36}  {:<20}  {:<10}  {:<20}",
            run.id.0,
            truncate(&run.name, 20),
            format_status(run.status),
            format_time(&run.created_at),
        );
    }

    println!();
    println!("{} run(s)", runs.len());
}

/// Print detailed information about a run and its steps.
pub fn print_run_details(run: &Run, steps: &[Step]) {
    println!("Run: {}", run.id);
    println!();
    println!("  Name:           {}", run.name);
    println!("  Name Source:    {}", run.name_source.as_str());
    println!("  Status:         {}", format_status(run.status));
    println!("  Workspace:      {}", run.workspace_root);
    println!("  Spec:           {}", run.spec_path);
    if let Some(ref plan) = run.plan_path {
        println!("  Plan:           {}", plan);
    }

    // Worktree info
    if let Some(ref wt) = run.worktree {
        println!();
        println!("  Worktree:");
        println!("    Base Branch:    {}", wt.base_branch);
        println!("    Run Branch:     {}", wt.run_branch);
        if let Some(ref target) = wt.merge_target_branch {
            println!("    Merge Target:   {}", target);
            println!("    Merge Strategy: {}", wt.merge_strategy.as_str());
        }
        println!("    Path:           {}", wt.worktree_path);
    }

    println!();
    println!("  Created:        {}", format_time(&run.created_at));
    println!("  Updated:        {}", format_time(&run.updated_at));

    // Steps
    if !steps.is_empty() {
        println!();
        println!("  Steps:");
        println!(
            "    {:<36}  {:<14}  {:<12}  {:<6}  {:<8}",
            "ID", "PHASE", "STATUS", "ATTEMPT", "EXIT"
        );
        println!("    {}", "-".repeat(80));

        for step in steps {
            let exit_code = step
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "    {:<36}  {:<14}  {:<12}  {:<6}  {:<8}",
                step.id.0,
                step.phase.as_str(),
                format_step_status(step.status),
                step.attempt,
                exit_code,
            );
        }
    }

    // Artifact paths (spec Section 7.2)
    println!();
    println!("  Artifacts:");
    let log_dir = format!("{}/logs/loop/run-{}", run.workspace_root, run.id);
    println!("    Log Dir:      {}", log_dir);
    println!("    Prompt:       {}/prompt.txt", log_dir);
    println!("    Summary:      {}/summary.json", log_dir);
    println!("    Report:       {}/report.tsv", log_dir);
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
