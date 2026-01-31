//! loopctl - CLI client for loopd
//!
//! Local control plane client for the orchestrator daemon.
//! See spec: specs/orchestrator-daemon.md Section 4.1

mod client;
mod render;

use clap::{Parser, Subcommand};
use client::{Client, ClientError};
use loop_core::types::{MergeStrategy, RunNameSource, RunStatus, WorktreeProvider};
use loop_core::Config;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::UNIX_EPOCH;

/// CLI client for the loopd orchestrator daemon.
#[derive(Parser)]
#[command(name = "loopctl")]
#[command(about = "Control plane for loopd agent loop orchestrator")]
#[command(version)]
struct Cli {
    /// Daemon address (default: http://127.0.0.1:7700)
    #[arg(long, global = true, env = "LOOPD_ADDR")]
    addr: Option<String>,

    /// Auth token for daemon API
    #[arg(long, global = true, env = "LOOPD_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a new run from a spec file
    Run {
        /// Path to the spec file
        spec: Option<PathBuf>,

        /// Path to the plan file (optional)
        plan: Option<PathBuf>,

        /// Config file path (overrides .loop/config)
        #[arg(long)]
        config: Option<PathBuf>,

        /// Explicit run name (overrides auto-naming)
        #[arg(long)]
        name: Option<String>,

        /// Name source: haiku or spec_slug
        #[arg(long, value_parser = parse_name_source)]
        name_source: Option<RunNameSource>,

        /// Model for haiku name generation
        #[arg(long, default_value = "haiku")]
        name_model: String,

        /// Use interactive spec picker (requires gum)
        #[arg(long)]
        pick: bool,

        /// Base branch for worktree
        #[arg(long)]
        base_branch: Option<String>,

        /// Prefix for run branch names
        #[arg(long)]
        run_branch_prefix: Option<String>,

        /// Target branch to merge into on completion
        #[arg(long)]
        merge_target: Option<String>,

        /// Merge strategy: none, merge, or squash
        #[arg(long, value_parser = parse_merge_strategy)]
        merge_strategy: Option<MergeStrategy>,

        /// Worktree path template
        #[arg(long)]
        worktree_path_template: Option<String>,

        /// Worktree provider: auto, worktrunk, or git
        #[arg(long, value_parser = parse_worktree_provider)]
        worktree_provider: Option<WorktreeProvider>,

        /// Path to Worktrunk CLI binary
        #[arg(long)]
        worktrunk_bin: Option<PathBuf>,

        /// Path to Worktrunk config file
        #[arg(long)]
        worktrunk_config: Option<PathBuf>,

        /// Copy ignored files when using Worktrunk provider
        #[arg(long)]
        worktrunk_copy_ignored: bool,
    },

    /// Show the prompt that would be sent (no daemon required)
    Prompt {
        /// Path to the spec file
        spec: Option<PathBuf>,

        /// Path to the plan file (optional)
        plan: Option<PathBuf>,

        /// Config file path (overrides .loop/config)
        #[arg(long)]
        config: Option<PathBuf>,

        /// Use interactive spec picker (requires gum)
        #[arg(long)]
        pick: bool,
    },

    /// List runs (optionally filter by status)
    List {
        /// Filter by status (PENDING, RUNNING, PAUSED, COMPLETED, FAILED, CANCELED)
        #[arg(long, value_parser = parse_run_status)]
        status: Option<RunStatus>,

        /// Show only runs for current workspace
        #[arg(long)]
        workspace: bool,
    },

    /// Show detailed information about a run
    Inspect {
        /// Run ID
        run_id: String,
    },

    /// Pause a running run
    Pause {
        /// Run ID
        run_id: String,
    },

    /// Resume a paused run
    Resume {
        /// Run ID
        run_id: String,
    },

    /// Cancel a run
    Cancel {
        /// Run ID
        run_id: String,
    },

    /// Retry a failed run by re-queuing it
    Retry {
        /// Run ID
        run_id: String,
    },

    /// List worktrees for a workspace
    Worktrees {
        /// Workspace path (defaults to current directory)
        #[arg(default_value = ".")]
        workspace: String,
    },

    /// Remove a worktree (cancels attached run if any)
    #[command(name = "worktree-rm")]
    WorktreeRm {
        /// Worktree path to remove
        path: String,

        /// Workspace path (defaults to current directory)
        #[arg(long, default_value = ".")]
        workspace: String,

        /// Force removal even with uncommitted changes
        #[arg(short, long)]
        force: bool,
    },

    /// Stream live output from a run
    Tail {
        /// Run ID
        run_id: String,

        /// Follow output (like tail -f)
        #[arg(short, long)]
        follow: bool,
    },

    /// Run postmortem analysis on a completed run
    Analyze {
        /// Run ID (omit for latest)
        run_id: Option<String>,

        /// Use the most recent run
        #[arg(long, conflicts_with = "run_id")]
        latest: bool,

        /// Model for analysis (default: opus)
        #[arg(long, default_value = "opus")]
        model: String,

        /// Only print prompts, do not run analysis
        #[arg(long)]
        prompt_only: bool,

        /// Override log directory
        #[arg(long)]
        log_dir: Option<PathBuf>,
    },
}

fn parse_name_source(s: &str) -> Result<RunNameSource, String> {
    match s.to_lowercase().as_str() {
        "haiku" => Ok(RunNameSource::Haiku),
        "spec_slug" => Ok(RunNameSource::SpecSlug),
        _ => Err(format!(
            "invalid name source '{}', expected: haiku, spec_slug",
            s
        )),
    }
}

fn parse_merge_strategy(s: &str) -> Result<MergeStrategy, String> {
    match s.to_lowercase().as_str() {
        "none" => Ok(MergeStrategy::None),
        "merge" => Ok(MergeStrategy::Merge),
        "squash" => Ok(MergeStrategy::Squash),
        _ => Err(format!(
            "invalid merge strategy '{}', expected: none, merge, squash",
            s
        )),
    }
}

fn parse_run_status(s: &str) -> Result<RunStatus, String> {
    match s.to_uppercase().as_str() {
        "PENDING" => Ok(RunStatus::Pending),
        "RUNNING" => Ok(RunStatus::Running),
        "PAUSED" => Ok(RunStatus::Paused),
        "COMPLETED" => Ok(RunStatus::Completed),
        "FAILED" => Ok(RunStatus::Failed),
        "CANCELED" => Ok(RunStatus::Canceled),
        _ => Err(format!(
            "invalid status '{}', expected: PENDING, RUNNING, PAUSED, COMPLETED, FAILED, CANCELED",
            s
        )),
    }
}

fn parse_worktree_provider(s: &str) -> Result<WorktreeProvider, String> {
    match s.to_lowercase().as_str() {
        "auto" => Ok(WorktreeProvider::Auto),
        "worktrunk" => Ok(WorktreeProvider::Worktrunk),
        "git" => Ok(WorktreeProvider::Git),
        _ => Err(format!(
            "invalid worktree provider '{}', expected: auto, worktrunk, git",
            s
        )),
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let addr = cli
        .addr
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let client = Client::new(&addr, cli.token.as_deref());

    let requires_daemon = !matches!(cli.command, Command::Prompt { .. });
    if requires_daemon {
        // Wait for daemon to be ready with exponential backoff (Section 4.1).
        // Retry window: 5s total, starting at 200ms backoff.
        if let Err(e) = client.wait_for_ready().await {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }

    let result = match cli.command {
        Command::Run {
            spec,
            plan,
            config,
            name,
            name_source,
            name_model,
            pick,
            base_branch,
            run_branch_prefix,
            merge_target,
            merge_strategy,
            worktree_path_template,
            worktree_provider,
            worktrunk_bin,
            worktrunk_config,
            worktrunk_copy_ignored,
        } => {
            run_create(
                &client,
                spec,
                plan,
                config,
                name,
                name_source,
                name_model,
                pick,
                base_branch,
                run_branch_prefix,
                merge_target,
                merge_strategy,
                worktree_path_template,
                worktree_provider,
                worktrunk_bin,
                worktrunk_config,
                worktrunk_copy_ignored,
            )
            .await
        }
        Command::Prompt {
            spec,
            plan,
            config,
            pick,
        } => show_prompt(spec, plan, config, pick),
        Command::List { status, workspace } => run_list(&client, status, workspace).await,
        Command::Inspect { run_id } => run_inspect(&client, &run_id).await,
        Command::Pause { run_id } => run_pause(&client, &run_id).await,
        Command::Resume { run_id } => run_resume(&client, &run_id).await,
        Command::Cancel { run_id } => run_cancel(&client, &run_id).await,
        Command::Retry { run_id } => run_retry(&client, &run_id).await,
        Command::Worktrees { workspace } => run_worktrees(&client, &workspace).await,
        Command::WorktreeRm {
            path,
            workspace,
            force,
        } => run_worktree_rm(&client, &workspace, &path, force).await,
        Command::Tail { run_id, follow } => run_tail(&client, &run_id, follow).await,
        Command::Analyze {
            run_id,
            latest,
            model,
            prompt_only,
            log_dir,
        } => run_analyze(&client, run_id, latest, &model, prompt_only, log_dir).await,
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

async fn run_create(
    client: &Client,
    spec: Option<PathBuf>,
    plan: Option<PathBuf>,
    config: Option<PathBuf>,
    name: Option<String>,
    name_source: Option<RunNameSource>,
    _name_model: String,
    pick: bool,
    base_branch: Option<String>,
    run_branch_prefix: Option<String>,
    merge_target: Option<String>,
    merge_strategy: Option<MergeStrategy>,
    worktree_path_template: Option<String>,
    worktree_provider: Option<WorktreeProvider>,
    worktrunk_bin: Option<PathBuf>,
    worktrunk_config: Option<PathBuf>,
    worktrunk_copy_ignored: bool,
) -> Result<(), ClientError> {
    let inputs = resolve_run_inputs(spec, plan, config, pick)?;

    let req = client::CreateRunRequest {
        spec_path: inputs.spec_path.to_string_lossy().to_string(),
        plan_path: inputs
            .plan_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        workspace_root: inputs.workspace_root.to_string_lossy().to_string(),
        config_override: inputs
            .config_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        name,
        name_source,
        base_branch,
        run_branch_prefix,
        merge_target_branch: merge_target,
        merge_strategy,
        worktree_path_template,
        worktree_provider,
        worktrunk_bin: worktrunk_bin.map(|p| p.to_string_lossy().to_string()),
        worktrunk_config_path: worktrunk_config.map(|p| p.to_string_lossy().to_string()),
        worktrunk_copy_ignored: if worktrunk_copy_ignored {
            Some(true)
        } else {
            None
        },
    };

    let run = client.create_run(req).await?;
    render::print_run_created(&run);
    Ok(())
}

async fn run_list(
    client: &Client,
    status: Option<RunStatus>,
    workspace: bool,
) -> Result<(), ClientError> {
    let workspace_root = if workspace {
        Some(find_workspace_root()?.to_string_lossy().to_string())
    } else {
        None
    };

    let runs = client.list_runs(status, workspace_root.as_deref()).await?;
    render::print_run_list(&runs);
    Ok(())
}

async fn run_inspect(client: &Client, run_id: &str) -> Result<(), ClientError> {
    let run = client.get_run(run_id).await?;
    let steps = client.list_steps(run_id).await?;
    render::print_run_details(&run, &steps);
    Ok(())
}

async fn run_pause(client: &Client, run_id: &str) -> Result<(), ClientError> {
    client.pause_run(run_id).await?;
    println!("Run {} paused", run_id);
    Ok(())
}

async fn run_resume(client: &Client, run_id: &str) -> Result<(), ClientError> {
    client.resume_run(run_id).await?;
    println!("Run {} resumed", run_id);
    Ok(())
}

async fn run_cancel(client: &Client, run_id: &str) -> Result<(), ClientError> {
    client.cancel_run(run_id).await?;
    println!("Run {} canceled", run_id);
    Ok(())
}

async fn run_retry(client: &Client, run_id: &str) -> Result<(), ClientError> {
    client.retry_run(run_id).await?;
    println!("Run {} re-queued", run_id);
    Ok(())
}

async fn run_worktrees(client: &Client, workspace: &str) -> Result<(), ClientError> {
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|e| ClientError::IoError(format!("invalid workspace path: {}", e)))?;
    let workspace_str = workspace.to_string_lossy();

    let response = client.list_worktrees(&workspace_str).await?;

    if response.worktrees.is_empty() {
        println!("No worktrees found for {}", workspace_str);
        return Ok(());
    }

    println!(
        "{:<60} {:<30} {:<12} {}",
        "PATH", "BRANCH", "RUN STATUS", "RUN ID"
    );
    println!("{}", "-".repeat(120));

    for wt in &response.worktrees {
        println!(
            "{:<60} {:<30} {:<12} {}",
            render::truncate(&wt.path, 58),
            wt.branch.as_deref().unwrap_or("-"),
            wt.run_status.as_deref().unwrap_or("-"),
            wt.run_id.as_deref().unwrap_or("-"),
        );
    }

    println!("\n{} worktree(s)", response.worktrees.len());
    Ok(())
}

async fn run_worktree_rm(
    client: &Client,
    workspace: &str,
    path: &str,
    force: bool,
) -> Result<(), ClientError> {
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|e| ClientError::IoError(format!("invalid workspace path: {}", e)))?;
    let worktree_path = std::fs::canonicalize(path)
        .map_err(|e| ClientError::IoError(format!("invalid worktree path: {}", e)))?;

    client
        .remove_worktree(
            &workspace.to_string_lossy(),
            &worktree_path.to_string_lossy(),
            force,
        )
        .await?;

    println!("Worktree {} removed", worktree_path.display());
    Ok(())
}

async fn run_tail(client: &Client, run_id: &str, follow: bool) -> Result<(), ClientError> {
    client.tail_run(run_id, follow).await
}

async fn run_analyze(
    client: &Client,
    run_id: Option<String>,
    latest: bool,
    model: &str,
    prompt_only: bool,
    log_dir: Option<PathBuf>,
) -> Result<(), ClientError> {
    // Resolve run ID
    let resolved_run_id = if let Some(id) = run_id {
        id
    } else if latest {
        // Find most recent run from daemon
        let runs = client.list_runs(None, None).await?;
        runs.into_iter()
            .max_by_key(|r| r.created_at)
            .map(|r| r.id.to_string())
            .ok_or_else(|| ClientError::IoError("no runs found".to_string()))?
    } else {
        // Default to latest if no ID provided
        let runs = client.list_runs(None, None).await?;
        runs.into_iter()
            .max_by_key(|r| r.created_at)
            .map(|r| r.id.to_string())
            .ok_or_else(|| ClientError::IoError("no runs found".to_string()))?
    };

    if prompt_only {
        // Generate and print prompts locally without running analysis
        let run = client.get_run(&resolved_run_id).await?;
        let prompts = generate_analysis_prompts(&run, log_dir.as_deref())?;
        for (name, prompt) in prompts {
            println!("=== {} ===\n{}\n", name, prompt);
        }
        Ok(())
    } else {
        // Request postmortem from daemon
        let result = client
            .trigger_postmortem(&resolved_run_id, model, false)
            .await?;
        println!("Postmortem {} for run {}", result.status, resolved_run_id);
        if !result.artifacts.is_empty() {
            println!("Artifacts:");
            for artifact in &result.artifacts {
                println!("  {}", artifact);
            }
        }
        Ok(())
    }
}

/// Generate analysis prompts for a run (spec §5.1 step 3).
fn generate_analysis_prompts(
    run: &loop_core::types::Run,
    log_dir_override: Option<&Path>,
) -> Result<Vec<(String, String)>, ClientError> {
    let workspace_root = Path::new(&run.workspace_root);
    let run_dir = if let Some(log_dir) = log_dir_override {
        log_dir.join(format!("run-{}", run.id))
    } else {
        loop_core::workspace_run_dir(workspace_root, &run.id)
    };

    let run_report = run_dir.join("report.tsv");
    let run_log = run_dir.join("run.log");
    let prompt_snapshot = run_dir.join("prompt.txt");
    let summary_json = run_dir.join("summary.json");

    // Parse run metadata from report if available
    let (last_iter, last_iter_log, last_iter_tail, completion_iter, completion_mode) =
        if run_report.exists() {
            parse_report_metadata(&run_report)?
        } else {
            (None, None, None, None, None)
        };

    let completion_display = match completion_iter {
        Some(iter) => {
            if let Some(mode) = &completion_mode {
                format!("iteration {} ({})", iter, mode)
            } else {
                format!("iteration {}", iter)
            }
        }
        None => "not detected".to_string(),
    };

    let run_model = resolve_run_model(run);

    // Build run quality prompt (matches bin/loop-analyze)
    let run_quality_prompt = format!(
        r#"Analyze this agent-loop run. Focus on end-of-task behavior, completion protocol compliance, and
actionable improvements to the spec templates and loop prompt.

Run metadata:
- Run ID: {}
- Completion detected: {}
- Last iteration observed: {}
- Model: {}

Artifacts (read all that exist):
- Run report (TSV): {}
- Run log: {}
- Prompt snapshot: {}
- Summary JSON: {}
- Last iteration tail: {}
- Last iteration log: {}

Return:
1) Short timeline summary + anomalies
2) End-of-task behavior (did it cleanly finish? protocol violations?)
3) Spec/template improvements (actionable)
4) Loop prompt improvements (actionable)
5) Loop UX/logging improvements (actionable)"#,
        run.id,
        completion_display,
        last_iter
            .map(|i| i.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        run_model,
        run_report.display(),
        run_log.display(),
        prompt_snapshot.display(),
        summary_json.display(),
        last_iter_tail.as_deref().unwrap_or("unknown"),
        last_iter_log.as_deref().unwrap_or("unknown"),
    );

    // Build spec compliance prompt
    let spec_path = &run.spec_path;
    let plan_path = run.plan_path.as_deref().unwrap_or("unknown");
    let analysis_dir = run_dir.join("analysis");

    let spec_compliance_prompt = format!(
        r#"Analyze the implementation against the spec and plan. Determine whether the spec is clear and whether
the implementation followed it. Highlight any changes required to fully reach the spec requirements.

Context:
- Spec: {}
- Plan: {}
- Model: {}

Artifacts (read all that exist):
- Spec: {}
- Plan: {}
- Git status: {}/git-status.txt
- Last commit summary: {}/git-last-commit.txt
- Last commit patch: {}/git-last-commit.patch
- Working tree diff: {}/git-diff.patch
- Run summary: {}

Return a Markdown report with sections:
1) Compliance summary (pass/fail + rationale)
2) Deviations (spec gap vs implementation deviation)
3) Missing verification steps
4) Required changes to meet the spec (bullet list)
5) Spec/template edits to prevent recurrence"#,
        spec_path,
        plan_path,
        run_model,
        spec_path,
        plan_path,
        analysis_dir.display(),
        analysis_dir.display(),
        analysis_dir.display(),
        analysis_dir.display(),
        summary_json.display(),
    );

    // Build summary prompt
    let summary_prompt = format!(
        r#"Synthesize the following reports into a final postmortem. Decide the primary root cause and provide
actionable changes to specs, prompt, and tooling.

Inputs:
- Spec compliance report: {}/spec-compliance.md
- Run quality report: {}/run-quality.md

Return a Markdown report with sections:
1) Root cause classification (spec gap vs implementation deviation vs execution failure)
2) Evidence (file/log references)
3) Required changes to reach the spec (bullet list)
4) Spec template changes
5) Loop prompt changes
6) Tooling/UX changes"#,
        analysis_dir.display(),
        analysis_dir.display(),
    );

    Ok(vec![
        ("run-quality".to_string(), run_quality_prompt),
        ("spec-compliance".to_string(), spec_compliance_prompt),
        ("summary".to_string(), summary_prompt),
    ])
}

/// Parse report TSV for iteration metadata.
fn parse_report_metadata(
    report_path: &Path,
) -> Result<
    (
        Option<u32>,
        Option<String>,
        Option<String>,
        Option<u32>,
        Option<String>,
    ),
    ClientError,
> {
    let content = std::fs::read_to_string(report_path)
        .map_err(|e| ClientError::IoError(format!("{}: {}", report_path.display(), e)))?;

    let mut last_iter: Option<u32> = None;
    let mut last_iter_log: Option<String> = None;
    let mut last_iter_tail: Option<String> = None;
    let mut completion_iter: Option<u32> = None;
    let mut completion_mode: Option<String> = None;

    for line in content.lines().skip(1) {
        // Skip header
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 9 {
            continue;
        }

        let kind = fields[1];
        let iteration: Option<u32> = fields[2].parse().ok();
        let output_path = fields[7];
        let message = fields.get(8).unwrap_or(&"");

        match kind {
            "ITERATION_END" => {
                last_iter = iteration;
                if !output_path.is_empty() {
                    last_iter_log = Some(output_path.to_string());
                }
            }
            "ITERATION_TAIL" => {
                if !output_path.is_empty() {
                    last_iter_tail = Some(output_path.to_string());
                }
            }
            "COMPLETE_DETECTED" => {
                completion_iter = iteration;
                if let Some(mode_str) = message.strip_prefix("mode=") {
                    completion_mode = Some(mode_str.to_string());
                }
            }
            _ => {}
        }
    }

    Ok((
        last_iter,
        last_iter_log,
        last_iter_tail,
        completion_iter,
        completion_mode,
    ))
}

fn resolve_run_model(run: &loop_core::types::Run) -> String {
    if let Some(config_json) = run.config_json.as_ref() {
        if let Ok(config) = serde_json::from_str::<Config>(config_json) {
            return config.model;
        }

        let config_path = Path::new(config_json);
        let resolved = if config_path.is_absolute() {
            config_path.to_path_buf()
        } else {
            Path::new(&run.workspace_root).join(config_path)
        };

        if resolved.exists() {
            if let Ok(config) = Config::from_file(&resolved) {
                return config.model;
            }
        }
    }

    "unknown".to_string()
}

fn show_prompt(
    spec: Option<PathBuf>,
    plan: Option<PathBuf>,
    config: Option<PathBuf>,
    pick: bool,
) -> Result<(), ClientError> {
    let inputs = resolve_run_inputs(spec, plan, config, pick)?;
    let prompt = build_prompt_preview(
        &inputs.spec_path,
        inputs.plan_path.as_deref(),
        &inputs.workspace_root,
        &inputs.config,
    );
    println!("{}", prompt);
    Ok(())
}

/// Find the workspace root (git root or cwd).
fn find_workspace_root() -> Result<PathBuf, ClientError> {
    // Try to find git root
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok(PathBuf::from(path))
        }
        _ => {
            // Fall back to current directory
            std::env::current_dir().map_err(|e| ClientError::IoError(e.to_string()))
        }
    }
}

#[derive(Debug, Clone)]
struct SpecEntry {
    spec_path: PathBuf,
    plan_path: PathBuf,
    title: String,
    status: Option<String>,
    last_updated: Option<String>,
    sort_key: u32,
}

#[derive(Debug)]
struct ResolvedRunInputs {
    workspace_root: PathBuf,
    config: Config,
    spec_path: PathBuf,
    plan_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
}

fn load_workspace_config(
    workspace_root: &Path,
    config_override: Option<&Path>,
) -> Result<Config, ClientError> {
    let mut config = Config::default();
    let project_config = workspace_root.join(".loop/config");
    if project_config.exists() {
        config
            .load_file(&project_config)
            .map_err(|e| ClientError::IoError(format!("{}: {}", project_config.display(), e)))?;
    }

    if let Some(override_path) = config_override {
        if override_path.exists() {
            config
                .load_file(override_path)
                .map_err(|e| ClientError::IoError(format!("{}: {}", override_path.display(), e)))?;
        } else {
            return Err(ClientError::IoError(format!(
                "config override not found: {}",
                override_path.display()
            )));
        }
    }

    config.resolve_paths(workspace_root);
    Ok(config)
}

fn resolve_run_inputs(
    spec: Option<PathBuf>,
    plan: Option<PathBuf>,
    config: Option<PathBuf>,
    pick: bool,
) -> Result<ResolvedRunInputs, ClientError> {
    let workspace_root = find_workspace_root()?;
    let config_path = config.map(|c| {
        if c.is_absolute() {
            c
        } else {
            workspace_root.join(&c)
        }
    });

    let config = load_workspace_config(&workspace_root, config_path.as_deref())?;

    let use_picker = pick || spec.is_none();
    let (spec_path, picked_plan_path) = if use_picker {
        let entry = pick_spec(&config)?;
        (entry.spec_path, Some(entry.plan_path))
    } else {
        (spec.expect("spec is required when not picking"), None)
    };

    let resolved_spec = resolve_existing_file(&spec_path, &[&config.specs_dir, &workspace_root])
        .ok_or_else(|| ClientError::IoError(format!("spec not found: {}", spec_path.display())))?;

    let plan_path: Option<PathBuf> = if let Some(plan) = plan {
        Some(
            resolve_existing_file(&plan, &[&config.plans_dir, &workspace_root]).ok_or_else(
                || ClientError::IoError(format!("plan not found: {}", plan.display())),
            )?,
        )
    } else if let Some(picked_plan) = picked_plan_path {
        if picked_plan.exists() {
            Some(picked_plan)
        } else {
            None
        }
    } else {
        let spec_base = resolved_spec
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let candidate = config.plans_dir.join(format!("{}-plan.md", spec_base));
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    };

    Ok(ResolvedRunInputs {
        workspace_root,
        config,
        spec_path: resolved_spec,
        plan_path,
        config_path,
    })
}

fn build_prompt_preview(
    spec_path: &Path,
    plan_path: Option<&Path>,
    workspace_root: &Path,
    config: &Config,
) -> String {
    let mut refs = format!("@{}", spec_path.display());
    if let Some(plan_path) = plan_path {
        refs.push_str(&format!(" @{}", plan_path.display()));
    }

    for context_path in &config.context_files {
        refs.push_str(&format!(" @{}", context_path.display()));
    }

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

    let custom_prompt = resolve_custom_prompt(config, workspace_root);
    let mut prompt = if let Some(custom_prompt) = custom_prompt {
        match std::fs::read_to_string(&custom_prompt) {
            Ok(content) => content,
            Err(err) => {
                eprintln!(
                    "warning: failed to read custom prompt {}: {}",
                    custom_prompt.display(),
                    err
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

    let plan_placeholder = plan_path
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    prompt = prompt
        .replace("SPEC_PATH", &spec_path.display().to_string())
        .replace("PLAN_PATH", &plan_placeholder);

    prompt
}

fn resolve_custom_prompt(config: &Config, workspace_root: &Path) -> Option<PathBuf> {
    if let Some(prompt_file) = config.prompt_file.as_ref() {
        if prompt_file.exists() {
            return Some(prompt_file.clone());
        }
        return None;
    }

    let default_prompt = workspace_root.join(".loop/prompt.txt");
    if default_prompt.exists() {
        Some(default_prompt)
    } else {
        None
    }
}

fn resolve_existing_file(path: &Path, bases: &[&Path]) -> Option<PathBuf> {
    if path.is_absolute() {
        if path.exists() {
            return Some(path.to_path_buf());
        }
        return None;
    }

    if path.exists() {
        return Some(path.to_path_buf());
    }

    for base in bases {
        let candidate = base.join(path);
        if candidate.exists() {
            return Some(candidate);
        }
        if let Some(name) = path.file_name() {
            let candidate = base.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn pick_spec(config: &Config) -> Result<SpecEntry, ClientError> {
    let entries = discover_specs(config)?;
    if entries.is_empty() {
        return Err(ClientError::IoError(format!(
            "no specs found in {}",
            config.specs_dir.display()
        )));
    }

    if !check_gum() {
        eprintln!("gum not found; pass a spec path instead");
        list_known_specs(&entries);
        return Err(ClientError::IoError(
            "gum is required for --pick".to_string(),
        ));
    }

    let mut display_lines = Vec::with_capacity(entries.len());
    for entry in &entries {
        display_lines.push(format_spec_display(entry));
    }

    let selected = run_gum_filter(&display_lines)?;
    let index = display_lines
        .iter()
        .position(|line| line == &selected)
        .ok_or_else(|| ClientError::IoError("selection lookup failed".to_string()))?;

    Ok(entries[index].clone())
}

fn discover_specs(config: &Config) -> Result<Vec<SpecEntry>, ClientError> {
    let mut entries = Vec::new();
    let spec_root = &config.specs_dir;

    let dir_entries = std::fs::read_dir(spec_root)
        .map_err(|e| ClientError::IoError(format!("{}: {}", spec_root.display(), e)))?;

    for entry in dir_entries {
        let entry = entry.map_err(|e| ClientError::IoError(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("README.md") {
            continue;
        }

        let entry = parse_spec_entry(&path, &config.plans_dir)?;
        entries.push(entry);
    }

    entries.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| a.title.cmp(&b.title))
    });

    Ok(entries)
}

fn parse_spec_entry(spec_path: &Path, plans_dir: &Path) -> Result<SpecEntry, ClientError> {
    let (title, status, last_updated) = read_spec_metadata(spec_path)?;
    let spec_base = spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let plan_path = plans_dir.join(format!("{}-plan.md", spec_base));

    let sort_key = last_updated
        .as_deref()
        .and_then(parse_date_key)
        .or_else(|| file_mtime_date_key(spec_path))
        .unwrap_or(0);

    Ok(SpecEntry {
        spec_path: spec_path.to_path_buf(),
        plan_path,
        title,
        status,
        last_updated,
        sort_key,
    })
}

fn read_spec_metadata(
    spec_path: &Path,
) -> Result<(String, Option<String>, Option<String>), ClientError> {
    let file = std::fs::File::open(spec_path)
        .map_err(|e| ClientError::IoError(format!("{}: {}", spec_path.display(), e)))?;
    let reader = std::io::BufReader::new(file);

    let mut title = None;
    let mut status = None;
    let mut last_updated = None;

    for line in reader.lines().take(20) {
        let line = line.map_err(|e| ClientError::IoError(e.to_string()))?;
        let trimmed = line.trim();

        if title.is_none() && trimmed.starts_with("# ") {
            title = Some(trimmed.trim_start_matches("# ").trim().to_string());
            continue;
        }
        if status.is_none() {
            if let Some(rest) = trimmed.strip_prefix("**Status:**") {
                let value = rest.trim();
                if !value.is_empty() {
                    status = Some(value.to_string());
                }
            }
        }
        if last_updated.is_none() {
            if let Some(rest) = trimmed.strip_prefix("**Last Updated:**") {
                let value = rest.trim();
                if !value.is_empty() {
                    last_updated = Some(value.to_string());
                }
            }
        }
        if title.is_some() && status.is_some() && last_updated.is_some() {
            break;
        }
    }

    let fallback_title = spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();

    Ok((title.unwrap_or(fallback_title), status, last_updated))
}

fn format_spec_display(entry: &SpecEntry) -> String {
    let mut display = String::new();
    if let Some(status) = &entry.status {
        display.push_str(&format!("[{}] ", status));
    }
    display.push_str(&entry.title);
    if let Some(last_updated) = &entry.last_updated {
        display.push_str(&format!(" ({})", last_updated));
    }
    let file_name = entry
        .spec_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    display.push_str(&format!(" - {}", file_name));
    display
}

fn list_known_specs(entries: &[SpecEntry]) {
    eprintln!("Known specs:");
    for entry in entries {
        eprintln!("  {} - {}", entry.spec_path.display(), entry.title);
    }
}

fn run_gum_filter(options: &[String]) -> Result<String, ClientError> {
    let input = options.join("\n");
    let mut child = ProcessCommand::new("gum")
        .args(["filter", "--placeholder", "Select a spec..."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| ClientError::IoError(e.to_string()))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| ClientError::IoError(e.to_string()))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| ClientError::IoError(e.to_string()))?;
    if !output.status.success() {
        return Err(ClientError::IoError("gum selection canceled".to_string()));
    }

    let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if selected.is_empty() {
        return Err(ClientError::IoError("no spec selected".to_string()));
    }
    Ok(selected)
}

fn check_gum() -> bool {
    ProcessCommand::new("gum")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn parse_date_key(value: &str) -> Option<u32> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return None;
    }
    let year: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    Some(year * 10000 + month * 100 + day)
}

fn file_mtime_date_key(path: &Path) -> Option<u32> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    let days = (duration.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    if year <= 0 {
        return None;
    }
    Some((year as u32) * 10000 + (month as u32) * 100 + day as u32)
}

fn civil_from_days(days: i64) -> (i32, i32, i32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as i32, d as i32)
}
