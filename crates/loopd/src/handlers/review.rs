//! Review workflow handlers for completed runs.
//!
//! Implements endpoints from daemon-review-api.md Section 4:
//! - GET /runs/{id}/diff - fetch commits and aggregate diff
//! - POST /runs/{id}/scrap - delete branch
//! - POST /runs/{id}/merge - merge to target branch
//! - POST /runs/{id}/create-pr - create GitHub PR

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use loop_core::{Id, MergeStrategy, ReviewStatus, RunStatus, RunWorktree};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::server::{check_auth, AppState, ErrorResponse};

// --- Response Types (daemon-review-api.md §3) ---

/// File diff information.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    pub status: String, // "added" | "modified" | "deleted" | "renamed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub patch: String,
    pub additions: u32,
    pub deletions: u32,
}

/// Commit information with diff.
#[derive(Debug, Serialize, Deserialize)]
pub struct DiffCommit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: String, // ISO 8601
    pub files: Vec<DiffFile>,
    pub stats: DiffStats,
}

/// Diff statistics.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DiffStats {
    pub additions: u32,
    pub deletions: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_changed: Option<u32>,
}

/// Response for GET /runs/{id}/diff.
#[derive(Debug, Serialize, Deserialize)]
pub struct RunDiffResponse {
    pub base_ref: String,
    pub head_ref: String,
    pub commits: Vec<DiffCommit>,
    pub files: Vec<DiffFile>,
    pub stats: DiffStats,
}

/// Snapshot payload stored for completed runs.
#[derive(Debug, Serialize, Deserialize)]
pub struct RunDiffSnapshot {
    pub base_ref: String,
    pub head_ref: String,
    pub base_sha: String,
    pub head_sha: String,
    pub commits: Vec<DiffCommit>,
    pub files: Vec<DiffFile>,
    pub stats: DiffStats,
}

impl RunDiffSnapshot {
    fn into_response(self) -> RunDiffResponse {
        RunDiffResponse {
            base_ref: self.base_ref,
            head_ref: self.head_ref,
            commits: self.commits,
            files: self.files,
            stats: self.stats,
        }
    }
}

/// Request for POST /runs/{id}/merge.
#[derive(Debug, Deserialize, Default)]
pub struct MergeRequest {
    #[serde(default)]
    pub strategy: Option<String>, // "squash" (default) | "merge"
}

/// Response for POST /runs/{id}/merge.
#[derive(Debug, Serialize)]
pub struct MergeResponse {
    pub commit: String,
}

/// Request for POST /runs/{id}/create-pr.
#[derive(Debug, Deserialize, Default)]
pub struct CreatePrRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

/// Response for POST /runs/{id}/create-pr.
#[derive(Debug, Serialize)]
pub struct CreatePrResponse {
    pub url: String,
}

// --- Handlers ---

/// GET /runs/{id}/diff - Get commits and aggregate diff for a run.
pub async fn get_run_diff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<RunDiffResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {e}"),
            }),
        )
    })?;

    // Verify run has worktree info.
    let worktree = run.worktree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "run has no worktree info".to_string(),
            }),
        )
    })?;

    // Prefer a stored snapshot if available.
    if let Ok(artifacts) = state.storage.list_artifacts(&run_id).await {
        if let Some(snapshot) = load_snapshot_from_artifacts(&artifacts) {
            return Ok(Json(snapshot.into_response()));
        }
    }

    let workspace_root = Path::new(&run.workspace_root);
    let diff = build_run_diff(workspace_root, worktree).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("git diff failed: {e}"),
            }),
        )
    })?;

    Ok(Json(diff))
}

/// POST /runs/{id}/scrap - Delete the run branch and mark as scrapped.
pub async fn scrap_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {e}"),
            }),
        )
    })?;

    // Verify run is in completed or failed state.
    if !matches!(run.status, RunStatus::Completed | RunStatus::Failed) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "run must be completed or failed to scrap (status: {})",
                    run.status.as_str()
                ),
            }),
        ));
    }

    let worktree = run.worktree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "run has no worktree info".to_string(),
            }),
        )
    })?;

    let workspace_root = Path::new(&run.workspace_root);
    let run_branch = &worktree.run_branch;

    // Delete the branch.
    delete_branch(workspace_root, run_branch).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("git branch -D failed: {e}"),
            }),
        )
    })?;

    // Update review status.
    state
        .storage
        .update_review_status(&run_id, ReviewStatus::Scrapped, None, None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to update review status: {e}"),
                }),
            )
        })?;

    info!("scrapped run: {} (branch {})", id, run_branch);
    Ok(StatusCode::NO_CONTENT)
}

/// POST /runs/{id}/merge - Merge run branch into target branch.
pub async fn merge_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<MergeRequest>,
) -> Result<Json<MergeResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {e}"),
            }),
        )
    })?;

    // Verify run is completed.
    if run.status != RunStatus::Completed {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "run must be completed to merge (status: {})",
                    run.status.as_str()
                ),
            }),
        ));
    }

    let worktree = run.worktree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "run has no worktree info".to_string(),
            }),
        )
    })?;

    let workspace_root = Path::new(&run.workspace_root);
    let run_branch = &worktree.run_branch;
    let target_branch = worktree
        .merge_target_branch
        .as_ref()
        .unwrap_or(&worktree.base_branch);

    // Determine merge strategy.
    let squash = req.strategy.as_deref() != Some("merge");

    // Get current branch to restore later.
    let original_branch = get_current_branch(workspace_root).ok();

    // Perform the merge.
    let commit = perform_merge(workspace_root, run_branch, target_branch, squash).map_err(|e| {
        // Try to restore original branch on failure.
        if let Some(branch) = &original_branch {
            let _ = checkout_branch(workspace_root, branch);
        }
        let status = if e.contains("CONFLICT") {
            StatusCode::CONFLICT
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (
            status,
            Json(ErrorResponse {
                error: format!("merge failed: {e}"),
            }),
        )
    })?;

    // Restore original branch.
    if let Some(branch) = original_branch {
        let _ = checkout_branch(workspace_root, &branch);
    }

    // Update review status.
    state
        .storage
        .update_review_status(&run_id, ReviewStatus::Merged, None, Some(&commit))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to update review status: {e}"),
                }),
            )
        })?;

    info!(
        "merged run: {} ({} -> {}, commit {})",
        id, run_branch, target_branch, commit
    );
    Ok(Json(MergeResponse { commit }))
}

/// POST /runs/{id}/create-pr - Create a GitHub PR from the run branch.
pub async fn create_pr(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<CreatePrRequest>,
) -> Result<Json<CreatePrResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {e}"),
            }),
        )
    })?;

    // Verify run is completed.
    if run.status != RunStatus::Completed {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!(
                    "run must be completed to create PR (status: {})",
                    run.status.as_str()
                ),
            }),
        ));
    }

    let worktree = run.worktree.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "run has no worktree info".to_string(),
            }),
        )
    })?;

    // Check if gh CLI is available.
    if !is_gh_available() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "gh CLI not available".to_string(),
            }),
        ));
    }

    let workspace_root = Path::new(&run.workspace_root);
    let run_branch = &worktree.run_branch;
    let target_branch = worktree
        .merge_target_branch
        .as_ref()
        .unwrap_or(&worktree.base_branch);

    // Default title and body.
    let title = req.title.unwrap_or_else(|| run.name.clone());
    let body = req
        .body
        .unwrap_or_else(|| format!("Created by loopd run {}", run.id));

    // Create the PR.
    let url = create_github_pr(workspace_root, run_branch, target_branch, &title, &body).map_err(
        |e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("gh pr create failed: {e}"),
                }),
            )
        },
    )?;

    // Update review status.
    state
        .storage
        .update_review_status(&run_id, ReviewStatus::PrCreated, Some(&url), None)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to update review status: {e}"),
                }),
            )
        })?;

    info!("created PR for run: {} ({})", id, url);
    Ok(Json(CreatePrResponse { url }))
}

// --- Git Helpers ---

/// Get commits between base and head refs.
fn get_commits(workspace_root: &Path, base: &str, head: &str) -> Result<Vec<DiffCommit>, String> {
    // git log base..head --format="%H|%s|%an|%aI"
    let output = Command::new("git")
        .args([
            "log",
            &format!("{base}..{head}"),
            "--format=%H|%s|%an|%aI",
            "--reverse",
        ])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }

        let sha = parts[0].to_string();
        let message = parts[1].to_string();
        let author = parts[2].to_string();
        let timestamp = parts[3].to_string();

        // Get per-commit diff.
        let (files, stats) = get_commit_diff(workspace_root, &sha)?;

        commits.push(DiffCommit {
            sha,
            message,
            author,
            timestamp,
            files,
            stats,
        });
    }

    Ok(commits)
}

/// Get diff for a single commit.
fn get_commit_diff(workspace_root: &Path, sha: &str) -> Result<(Vec<DiffFile>, DiffStats), String> {
    // git show <sha> --numstat --format=""
    let numstat_output = Command::new("git")
        .args(["show", sha, "--numstat", "--format="])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !numstat_output.status.success() {
        let stderr = String::from_utf8_lossy(&numstat_output.stderr);
        return Err(stderr.to_string());
    }

    let numstat = String::from_utf8_lossy(&numstat_output.stdout);
    let file_stats = parse_numstat(&numstat);

    // Get patches for each file.
    let mut files = Vec::new();
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;

    for (path, additions, deletions) in file_stats {
        let patch = get_file_patch(workspace_root, &format!("{sha}^"), sha, &path)?;
        let status = determine_file_status(workspace_root, sha, &path);

        total_additions += additions;
        total_deletions += deletions;

        files.push(DiffFile {
            path,
            status,
            old_path: None,
            patch,
            additions,
            deletions,
        });
    }

    let files_changed = files.len() as u32;
    Ok((
        files,
        DiffStats {
            additions: total_additions,
            deletions: total_deletions,
            files_changed: Some(files_changed),
        },
    ))
}

/// Get aggregate diff between base and head.
fn get_aggregate_diff(
    workspace_root: &Path,
    base: &str,
    head: &str,
) -> Result<(Vec<DiffFile>, DiffStats), String> {
    // git diff base...head --numstat
    let numstat_output = Command::new("git")
        .args(["diff", &format!("{base}...{head}"), "--numstat"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !numstat_output.status.success() {
        let stderr = String::from_utf8_lossy(&numstat_output.stderr);
        return Err(stderr.to_string());
    }

    let numstat = String::from_utf8_lossy(&numstat_output.stdout);
    let file_stats = parse_numstat(&numstat);

    let mut files = Vec::new();
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;

    for (path, additions, deletions) in file_stats {
        let patch = get_file_patch(workspace_root, base, head, &path)?;
        let status = determine_aggregate_file_status(workspace_root, base, head, &path);

        total_additions += additions;
        total_deletions += deletions;

        files.push(DiffFile {
            path,
            status,
            old_path: None,
            patch,
            additions,
            deletions,
        });
    }

    let files_changed = files.len() as u32;
    Ok((
        files,
        DiffStats {
            additions: total_additions,
            deletions: total_deletions,
            files_changed: Some(files_changed),
        },
    ))
}

/// Build the full diff response for a run.
pub fn build_run_diff(
    workspace_root: &Path,
    worktree: &RunWorktree,
) -> Result<RunDiffResponse, String> {
    let worktree_path = Path::new(&worktree.worktree_path);
    let base_ref = &worktree.base_branch;
    let head_ref = resolve_head_ref(workspace_root, worktree)?;

    let commits = get_commits(workspace_root, base_ref, &head_ref)?;
    let use_worktree = worktree_path.exists()
        && worktree_path.is_dir()
        && head_ref == worktree.run_branch;
    let (files, stats) = if use_worktree {
        get_worktree_diff(worktree_path, base_ref)?
    } else {
        get_aggregate_diff(workspace_root, base_ref, &head_ref)?
    };

    Ok(RunDiffResponse {
        base_ref: base_ref.clone(),
        head_ref,
        commits,
        files,
        stats,
    })
}

fn resolve_head_ref(workspace_root: &Path, worktree: &RunWorktree) -> Result<String, String> {
    let run_branch = &worktree.run_branch;
    let merge_target = worktree.merge_target_branch.as_deref();
    let merge_configured = merge_target.is_some() && worktree.merge_strategy != MergeStrategy::None;

    if merge_configured {
        if let Some(target) = merge_target {
            if branch_exists(workspace_root, target)? {
                return Ok(target.to_string());
            }
        }
    }

    if branch_exists(workspace_root, run_branch)? {
        return Ok(run_branch.clone());
    }

    if let Some(target) = merge_target {
        if branch_exists(workspace_root, target)? {
            return Ok(target.to_string());
        }
    }

    let mut message = format!("branch not found: {run_branch}");
    if let Some(target) = merge_target {
        message.push_str(" (or ");
        message.push_str(target);
        message.push(')');
    }
    Err(message)
}

fn branch_exists(workspace_root: &Path, branch: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("refs/heads/{branch}")])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    Ok(output.status.success())
}

/// Build a diff snapshot payload for durable review.
pub fn build_run_diff_snapshot(
    workspace_root: &Path,
    worktree: &RunWorktree,
) -> Result<RunDiffSnapshot, String> {
    let diff = build_run_diff(workspace_root, worktree)?;
    let base_sha = get_ref_sha(workspace_root, &diff.base_ref)?;
    let head_sha = get_ref_sha(workspace_root, &diff.head_ref)?;

    Ok(RunDiffSnapshot {
        base_ref: diff.base_ref,
        head_ref: diff.head_ref,
        base_sha,
        head_sha,
        commits: diff.commits,
        files: diff.files,
        stats: diff.stats,
    })
}

/// Get aggregate diff from base ref to worktree state (includes uncommitted and untracked changes).
fn get_worktree_diff(
    worktree_path: &Path,
    base: &str,
) -> Result<(Vec<DiffFile>, DiffStats), String> {
    // git diff <base> --numstat (tracked changes)
    let numstat_output = Command::new("git")
        .args(["diff", base, "--numstat"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !numstat_output.status.success() {
        let stderr = String::from_utf8_lossy(&numstat_output.stderr);
        return Err(stderr.to_string());
    }

    let numstat = String::from_utf8_lossy(&numstat_output.stdout);
    let file_stats = parse_numstat(&numstat);

    let mut files = Vec::new();
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;

    for (path, additions, deletions) in file_stats {
        let patch = get_worktree_file_patch(worktree_path, base, &path)?;
        let status = determine_worktree_file_status(worktree_path, base, &path);

        total_additions += additions;
        total_deletions += deletions;

        files.push(DiffFile {
            path,
            status,
            old_path: None,
            patch,
            additions,
            deletions,
        });
    }

    // Include untracked files (new files not yet git-added).
    if let Ok(untracked) = get_untracked_files(worktree_path) {
        for path in untracked {
            if let Ok(diff_file) = build_untracked_diff(worktree_path, &path) {
                total_additions += diff_file.additions;
                files.push(diff_file);
            }
        }
    }

    let files_changed = files.len() as u32;
    Ok((
        files,
        DiffStats {
            additions: total_additions,
            deletions: total_deletions,
            files_changed: Some(files_changed),
        },
    ))
}

/// List untracked files in a worktree (respects .gitignore).
fn get_untracked_files(worktree_path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| !l.is_empty()).map(String::from).collect())
}

/// Build a DiffFile for an untracked file by reading its content and generating a unified diff patch.
fn build_untracked_diff(worktree_path: &Path, path: &str) -> Result<DiffFile, String> {
    // Use git diff --no-index to generate a proper unified diff for the untracked file.
    let output = Command::new("git")
        .args(["diff", "--no-index", "--", "/dev/null", path])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| e.to_string())?;

    // --no-index exits 1 when there are differences (which there always are vs /dev/null).
    let patch = String::from_utf8_lossy(&output.stdout).to_string();

    let additions = patch.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count() as u32;

    Ok(DiffFile {
        path: path.to_string(),
        status: "added".to_string(),
        old_path: None,
        patch,
        additions,
        deletions: 0,
    })
}

/// Parse git diff --numstat output.
fn parse_numstat(output: &str) -> Vec<(String, u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            (parts.len() >= 3).then(|| {
                let additions = parts[0].parse().unwrap_or(0);
                let deletions = parts[1].parse().unwrap_or(0);
                let path = parts[2].to_string();
                (path, additions, deletions)
            })
        })
        .collect()
}

/// Get unified diff patch for a single file.
fn get_file_patch(
    workspace_root: &Path,
    from: &str,
    to: &str,
    path: &str,
) -> Result<String, String> {
    let output = Command::new("git")
        .args(["diff", from, to, "--", path])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Get unified diff patch for a single file between base ref and worktree state.
fn get_worktree_file_patch(worktree_path: &Path, base: &str, path: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["diff", base, "--", path])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn get_ref_sha(workspace_root: &Path, ref_name: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["rev-parse", ref_name])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn load_snapshot_from_artifacts(artifacts: &[loop_core::Artifact]) -> Option<RunDiffSnapshot> {
    let mut candidates: Vec<&loop_core::Artifact> = artifacts
        .iter()
        .filter(|artifact| artifact.kind == "review_diff")
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by_key(|artifact| artifact.location.as_str() != "workspace");
    for artifact in candidates {
        if let Ok(content) = std::fs::read_to_string(&artifact.path) {
            if let Ok(snapshot) = serde_json::from_str::<RunDiffSnapshot>(&content) {
                return Some(snapshot);
            }
        }
    }

    None
}

/// Determine file status for a commit.
fn determine_file_status(workspace_root: &Path, sha: &str, path: &str) -> String {
    let output = Command::new("git")
        .args(["show", sha, "--name-status", "--format=", "--", path])
        .current_dir(workspace_root)
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(status) = line.chars().next() {
                return match status {
                    'A' => "added",
                    'D' => "deleted",
                    'M' => "modified",
                    'R' => "renamed",
                    _ => "modified",
                }
                .to_string();
            }
        }
    }
    "modified".to_string()
}

/// Determine aggregate file status between two refs.
fn determine_aggregate_file_status(
    workspace_root: &Path,
    base: &str,
    head: &str,
    path: &str,
) -> String {
    let output = Command::new("git")
        .args([
            "diff",
            &format!("{base}...{head}"),
            "--name-status",
            "--",
            path,
        ])
        .current_dir(workspace_root)
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(status) = line.chars().next() {
                return match status {
                    'A' => "added",
                    'D' => "deleted",
                    'M' => "modified",
                    'R' => "renamed",
                    _ => "modified",
                }
                .to_string();
            }
        }
    }
    "modified".to_string()
}

/// Determine file status between base ref and worktree state.
fn determine_worktree_file_status(worktree_path: &Path, base: &str, path: &str) -> String {
    let output = Command::new("git")
        .args(["diff", base, "--name-status", "--", path])
        .current_dir(worktree_path)
        .output();

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(status) = line.chars().next() {
                return match status {
                    'A' => "added",
                    'D' => "deleted",
                    'M' => "modified",
                    'R' => "renamed",
                    _ => "modified",
                }
                .to_string();
            }
        }
    }
    "modified".to_string()
}

/// Delete a branch.
fn delete_branch(workspace_root: &Path, branch: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["branch", "-D", branch])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(())
}

/// Get current branch.
fn get_current_branch(workspace_root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Checkout a branch.
fn checkout_branch(workspace_root: &Path, branch: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["checkout", branch])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(())
}

/// Perform merge (squash or regular).
fn perform_merge(
    workspace_root: &Path,
    source_branch: &str,
    target_branch: &str,
    squash: bool,
) -> Result<String, String> {
    // Checkout target branch.
    checkout_branch(workspace_root, target_branch)?;

    if squash {
        // Squash merge.
        let merge_output = Command::new("git")
            .args(["merge", "--squash", source_branch])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| e.to_string())?;

        if !merge_output.status.success() {
            let stderr = String::from_utf8_lossy(&merge_output.stderr);
            // Reset on failure.
            let _ = Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .current_dir(workspace_root)
                .output();
            return Err(stderr.to_string());
        }

        // Commit the squashed changes.
        let commit_msg = format!("Squash merge from {source_branch}");
        let commit_output = Command::new("git")
            .args(["commit", "-m", &commit_msg])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| e.to_string())?;

        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            // Check if nothing to commit (already in sync).
            if stderr.contains("nothing to commit") {
                // Get current HEAD as the "merge commit".
                return get_head_commit(workspace_root);
            }
            return Err(stderr.to_string());
        }
    } else {
        // Regular merge.
        let merge_output = Command::new("git")
            .args(["merge", source_branch, "--no-edit"])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| e.to_string())?;

        if !merge_output.status.success() {
            let stderr = String::from_utf8_lossy(&merge_output.stderr);
            // Abort merge on failure.
            let _ = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(workspace_root)
                .output();
            return Err(stderr.to_string());
        }
    }

    // Get the commit SHA.
    get_head_commit(workspace_root)
}

/// Get HEAD commit SHA.
fn get_head_commit(workspace_root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check if gh CLI is available.
fn is_gh_available() -> bool {
    Command::new("which")
        .arg("gh")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a GitHub PR using gh CLI.
fn create_github_pr(
    workspace_root: &Path,
    head_branch: &str,
    base_branch: &str,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "create",
            "--head",
            head_branch,
            "--base",
            base_branch,
            "--title",
            title,
            "--body",
            body,
        ])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    // Parse PR URL from stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let url = stdout.trim().to_string();

    if url.is_empty() {
        return Err("gh pr create returned empty output".to_string());
    }

    Ok(url)
}
