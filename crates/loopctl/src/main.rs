//! loopctl - CLI client for loopd
//!
//! Local control plane client for the orchestrator daemon.
//! See spec: specs/orchestrator-daemon.md Section 4.1

mod client;
mod render;

use clap::{Parser, Subcommand};
use client::{Client, ClientError};
use loop_core::types::{MergeStrategy, RunNameSource, RunStatus};
use std::path::PathBuf;

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
        spec: PathBuf,

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

    /// Stream live output from a run
    Tail {
        /// Run ID
        run_id: String,

        /// Follow output (like tail -f)
        #[arg(short, long)]
        follow: bool,
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let addr = cli
        .addr
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let client = Client::new(&addr, cli.token.as_deref());

    let result = match cli.command {
        Command::Run {
            spec,
            plan,
            config,
            name,
            name_source,
            name_model,
            base_branch,
            run_branch_prefix,
            merge_target,
            merge_strategy,
            worktree_path_template,
        } => {
            run_create(
                &client,
                spec,
                plan,
                config,
                name,
                name_source,
                name_model,
                base_branch,
                run_branch_prefix,
                merge_target,
                merge_strategy,
                worktree_path_template,
            )
            .await
        }
        Command::List { status, workspace } => run_list(&client, status, workspace).await,
        Command::Inspect { run_id } => run_inspect(&client, &run_id).await,
        Command::Pause { run_id } => run_pause(&client, &run_id).await,
        Command::Resume { run_id } => run_resume(&client, &run_id).await,
        Command::Cancel { run_id } => run_cancel(&client, &run_id).await,
        Command::Tail { run_id, follow } => run_tail(&client, &run_id, follow).await,
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

async fn run_create(
    client: &Client,
    spec: PathBuf,
    plan: Option<PathBuf>,
    config: Option<PathBuf>,
    name: Option<String>,
    name_source: Option<RunNameSource>,
    _name_model: String,
    base_branch: Option<String>,
    run_branch_prefix: Option<String>,
    merge_target: Option<String>,
    merge_strategy: Option<MergeStrategy>,
    worktree_path_template: Option<String>,
) -> Result<(), ClientError> {
    // Resolve workspace root (find git root or use cwd)
    let workspace_root = find_workspace_root()?;

    // Resolve spec path to absolute
    let spec_path = if spec.is_absolute() {
        spec
    } else {
        workspace_root.join(&spec)
    };

    // Resolve plan path if provided
    let plan_path = plan.map(|p| {
        if p.is_absolute() {
            p
        } else {
            workspace_root.join(&p)
        }
    });

    // Resolve config path if provided
    let config_path = config.map(|c| {
        if c.is_absolute() {
            c
        } else {
            workspace_root.join(&c)
        }
    });

    let req = client::CreateRunRequest {
        spec_path: spec_path.to_string_lossy().to_string(),
        plan_path: plan_path.map(|p| p.to_string_lossy().to_string()),
        workspace_root: workspace_root.to_string_lossy().to_string(),
        config_path: config_path.map(|p| p.to_string_lossy().to_string()),
        name,
        name_source,
        base_branch,
        run_branch_prefix,
        merge_target_branch: merge_target,
        merge_strategy,
        worktree_path_template,
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

async fn run_tail(client: &Client, run_id: &str, follow: bool) -> Result<(), ClientError> {
    client.tail_run(run_id, follow).await
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
