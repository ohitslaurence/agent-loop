//! Git operations for the orchestrator daemon.
//!
//! Implements worktree creation and branch naming defaults.
//! See spec Section 3 (Worktree fields) and Section 5 (Worktree + Merge Flow).

use loop_core::config::Config;
use loop_core::prompt::sanitize_branch_name;
use loop_core::types::{MergeStrategy, RunWorktree, WorktreeProvider};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("not a git repository: {0}")]
    NotARepo(String),
    #[error("failed to execute git: {0}")]
    Execution(#[from] std::io::Error),
    #[error("invalid utf-8 in git output")]
    InvalidUtf8,
    #[error("merge conflict: {0}")]
    MergeConflict(String),
    #[error("dirty working tree: {0}")]
    DirtyWorkingTree(String),
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Detect the default branch for a repository.
///
/// Tries `git symbolic-ref refs/remotes/origin/HEAD` first (tracks remote default),
/// then falls back to `main`.
pub fn detect_default_branch(workspace_root: &Path) -> Result<String> {
    // Try to get remote HEAD reference.
    let output = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(workspace_root)
        .output()?;

    if output.status.success() {
        let full_ref = String::from_utf8(output.stdout)
            .map_err(|_| GitError::InvalidUtf8)?
            .trim()
            .to_string();
        // Extract branch name from refs/remotes/origin/<branch>.
        if let Some(branch) = full_ref.strip_prefix("refs/remotes/origin/") {
            return Ok(branch.to_string());
        }
    }

    // Try to check if `main` exists.
    let main_check = Command::new("git")
        .args(["rev-parse", "--verify", "refs/heads/main"])
        .current_dir(workspace_root)
        .output()?;

    if main_check.status.success() {
        return Ok("main".to_string());
    }

    // Try `master` as final fallback.
    let master_check = Command::new("git")
        .args(["rev-parse", "--verify", "refs/heads/master"])
        .current_dir(workspace_root)
        .output()?;

    if master_check.status.success() {
        return Ok("master".to_string());
    }

    // Default to main even if it doesn't exist yet.
    Ok("main".to_string())
}

/// Get the repository directory name from workspace root.
pub fn repo_name(workspace_root: &Path) -> String {
    workspace_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string()
}

/// Expand the worktree path template.
///
/// Template variables (from spec Section 3):
/// - `{{ repo }}`: repository directory name
/// - `{{ run_branch }}`: full run branch name
/// - `{{ run_branch | sanitize }}`: filesystem-safe branch (slashes replaced with `-`)
pub fn expand_worktree_template(template: &str, workspace_root: &Path, run_branch: &str) -> String {
    let repo = repo_name(workspace_root);
    let sanitized = sanitize_branch_name(run_branch);

    template
        .replace("{{ repo }}", &repo)
        .replace("{{repo}}", &repo)
        .replace("{{ run_branch | sanitize }}", &sanitized)
        .replace("{{run_branch | sanitize}}", &sanitized)
        .replace("{{ branch | sanitize }}", &sanitized)
        .replace("{{branch | sanitize}}", &sanitized)
        .replace("{{ run_branch }}", run_branch)
        .replace("{{run_branch}}", run_branch)
        .replace("{{ branch }}", run_branch)
        .replace("{{branch}}", run_branch)
}

/// Resolve the worktree path to an absolute path.
///
/// If the expanded path is relative, resolve it relative to the workspace root's parent.
pub fn resolve_worktree_path(expanded: &str, workspace_root: &Path) -> std::path::PathBuf {
    let path = std::path::Path::new(expanded);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Resolve relative to workspace root (handles `../...` patterns).
        workspace_root
            .join(expanded)
            .canonicalize()
            .unwrap_or_else(|_| {
                // If canonicalize fails (path doesn't exist yet), normalize manually.
                normalize_path(&workspace_root.join(expanded))
            })
    }
}

/// Normalize a path by resolving `.` and `..` components.
fn normalize_path(path: &Path) -> std::path::PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

/// Build RunWorktree configuration from config and run parameters.
///
/// Applies defaults from spec Section 3:
/// - `base_branch`: detected default branch (fallback to `main`)
/// - `run_branch`: `<prefix><run_name_slug>` where prefix defaults to `run/`
/// - `merge_target_branch`: from config (optional)
/// - `merge_strategy`: from config, but only if merge_target_branch is set
/// - `worktree_path`: expanded from template
pub fn build_worktree_config(
    config: &Config,
    workspace_root: &Path,
    run_name: &str,
    _spec_path: &Path,
) -> Result<RunWorktree> {
    // Detect or use configured base branch.
    let base_branch = config.base_branch.clone().unwrap_or_else(|| {
        detect_default_branch(workspace_root).unwrap_or_else(|_| "main".to_string())
    });

    // Generate run branch from name.
    let run_name_slug = slugify(run_name);
    let run_branch = format!("{}{}", config.run_branch_prefix, run_name_slug);

    // Merge target from config (optional per spec).
    let merge_target_branch = config.merge_target_branch.clone();

    // Merge strategy: only meaningful when merge_target_branch is set.
    let merge_strategy = if merge_target_branch.is_some() {
        config.merge_strategy
    } else {
        MergeStrategy::None
    };

    // Expand worktree path template.
    let expanded =
        expand_worktree_template(&config.worktree_path_template, workspace_root, &run_branch);
    let worktree_path = resolve_worktree_path(&expanded, workspace_root);

    Ok(RunWorktree {
        base_branch,
        run_branch,
        merge_target_branch,
        merge_strategy,
        worktree_path: worktree_path.to_string_lossy().to_string(),
        // Default to Git provider; this will be overridden by provider resolution.
        provider: WorktreeProvider::Git,
    })
}

/// Create a slug from a run name (lowercase, alphanumeric, hyphens).
fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        // Collapse multiple hyphens.
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Check if a branch exists locally.
pub fn branch_exists(workspace_root: &Path, branch: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("refs/heads/{}", branch)])
        .current_dir(workspace_root)
        .output()?;

    Ok(output.status.success())
}

/// Create a new branch from base without checking it out.
pub fn create_branch(workspace_root: &Path, branch: &str, base: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["branch", branch, base])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git branch {} {}: {}",
            branch, base, stderr
        )));
    }

    Ok(())
}

/// Create a git worktree at the specified path for the given branch.
///
/// Creates the branch from base_branch if it doesn't exist.
pub fn create_worktree(
    workspace_root: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: &str,
) -> Result<()> {
    // Ensure parent directory exists.
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            GitError::CommandFailed(format!("failed to create parent directory: {}", e))
        })?;
    }

    // Check if branch exists; if not, create it from base.
    if !branch_exists(workspace_root, branch)? {
        create_branch(workspace_root, branch, base_branch)?;
    }

    // Create the worktree.
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_string_lossy().as_ref(),
            branch,
        ])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git worktree add: {}",
            stderr
        )));
    }

    Ok(())
}

/// Remove a git worktree.
pub fn remove_worktree(workspace_root: &Path, worktree_path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args([
            "worktree",
            "remove",
            worktree_path.to_string_lossy().as_ref(),
        ])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git worktree remove: {}",
            stderr
        )));
    }

    Ok(())
}

/// Force remove a git worktree (even with local changes).
pub fn remove_worktree_force(workspace_root: &Path, worktree_path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_string_lossy().as_ref(),
        ])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git worktree remove --force: {}",
            stderr
        )));
    }

    Ok(())
}

/// Information about a git worktree.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub commit: String,
}

/// List all git worktrees for a repository.
pub fn list_worktrees(workspace_root: &Path) -> Result<Vec<WorktreeInfo>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git worktree list: {}",
            stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_commit: Option<String> = None;
    let mut current_branch: Option<String> = None;

    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            // Save previous worktree if we have one
            if let (Some(path), Some(commit)) = (current_path.take(), current_commit.take()) {
                worktrees.push(WorktreeInfo {
                    path,
                    commit,
                    branch: current_branch.take(),
                });
            }
            current_path = Some(path.to_string());
        } else if let Some(commit) = line.strip_prefix("HEAD ") {
            current_commit = Some(commit.to_string());
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_string());
        }
    }

    // Don't forget the last worktree
    if let (Some(path), Some(commit)) = (current_path, current_commit) {
        worktrees.push(WorktreeInfo {
            path,
            commit,
            branch: current_branch,
        });
    }

    Ok(worktrees)
}

/// Check if the working tree is clean (no uncommitted changes).
pub fn is_working_tree_clean(workspace_root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!("git status: {}", stderr)));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|_| GitError::InvalidUtf8)?;
    Ok(stdout.trim().is_empty())
}

/// Checkout a branch in the workspace.
pub fn checkout_branch(workspace_root: &Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["checkout", branch])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::CommandFailed(format!(
            "git checkout {}: {}",
            branch, stderr
        )));
    }

    Ok(())
}

/// Merge a source branch into the current branch using regular merge.
///
/// Returns an error if there are merge conflicts.
pub fn merge_branch(workspace_root: &Path, source_branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["merge", source_branch, "--no-edit"])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check for conflict indicators.
        if stderr.contains("CONFLICT") || stderr.contains("Automatic merge failed") {
            // Abort the merge to leave the tree clean.
            let _ = Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(workspace_root)
                .output();
            return Err(GitError::MergeConflict(format!(
                "merge from {} failed: {}",
                source_branch, stderr
            )));
        }
        return Err(GitError::CommandFailed(format!(
            "git merge {}: {}",
            source_branch, stderr
        )));
    }

    Ok(())
}

/// Squash merge a source branch into the current branch.
///
/// Creates a single commit with all changes from the source branch.
/// Returns an error if there are merge conflicts.
pub fn squash_merge_branch(workspace_root: &Path, source_branch: &str) -> Result<()> {
    // First, do a squash merge (stages changes but doesn't commit).
    let output = Command::new("git")
        .args(["merge", "--squash", source_branch])
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("Automatic merge failed") {
            // Reset to clean state.
            let _ = Command::new("git")
                .args(["reset", "--hard", "HEAD"])
                .current_dir(workspace_root)
                .output();
            return Err(GitError::MergeConflict(format!(
                "squash merge from {} failed: {}",
                source_branch, stderr
            )));
        }
        return Err(GitError::CommandFailed(format!(
            "git merge --squash {}: {}",
            source_branch, stderr
        )));
    }

    // Check if there are changes to commit.
    let status_output = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(workspace_root)
        .output()?;

    if !status_output.status.success() {
        // There are staged changes; commit them.
        let commit_msg = format!("Squash merge from {}", source_branch);
        let commit_output = Command::new("git")
            .args(["commit", "-m", &commit_msg])
            .current_dir(workspace_root)
            .output()?;

        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            return Err(GitError::CommandFailed(format!(
                "git commit after squash: {}",
                stderr
            )));
        }
    }
    // If no changes, the branches were already in sync; nothing to commit.

    Ok(())
}

/// Perform the merge-to-target flow on run completion.
///
/// Implements spec Section 5.2 Worktree + Merge Flow step 3:
/// 1. Ensure target branch exists (create from base if missing)
/// 2. Merge or squash from run_branch into merge_target_branch
/// 3. Leave merge_target_branch checked out in the primary worktree
///
/// Does NOT push or open PR (v0.1 spec).
pub fn merge_to_target(
    workspace_root: &Path,
    run_branch: &str,
    merge_target_branch: &str,
    base_branch: &str,
    strategy: MergeStrategy,
) -> Result<()> {
    // Skip if strategy is None.
    if strategy == MergeStrategy::None {
        return Ok(());
    }

    // Check for clean working tree.
    if !is_working_tree_clean(workspace_root)? {
        return Err(GitError::DirtyWorkingTree(
            "cannot merge with uncommitted changes".to_string(),
        ));
    }

    // Ensure target branch exists; create from base if missing.
    if !branch_exists(workspace_root, merge_target_branch)? {
        create_branch(workspace_root, merge_target_branch, base_branch)?;
    }

    // Checkout the target branch.
    checkout_branch(workspace_root, merge_target_branch)?;

    // Perform the merge based on strategy.
    let result = match strategy {
        MergeStrategy::Merge => merge_branch(workspace_root, run_branch),
        MergeStrategy::Squash => squash_merge_branch(workspace_root, run_branch),
        MergeStrategy::None => Ok(()),
    };

    // On error, leave target branch checked out but with run_branch intact.
    if let Err(e) = &result {
        // Try to return to a clean state, but keep run_branch for recovery.
        tracing::warn!(
            "merge failed, run_branch {} preserved for manual recovery: {}",
            run_branch,
            e
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Create a test git repository.
    fn setup_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // Create initial commit so we have a valid HEAD.
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn test_repo_name() {
        assert_eq!(repo_name(Path::new("/home/user/my-project")), "my-project");
        assert_eq!(repo_name(Path::new("/workspace")), "workspace");
    }

    #[test]
    fn test_expand_worktree_template_default() {
        let template = "../{{ repo }}.{{ run_branch | sanitize }}";
        let expanded = expand_worktree_template(
            template,
            Path::new("/home/user/my-project"),
            "run/feature-x",
        );
        assert_eq!(expanded, "../my-project.run-feature-x");
    }

    #[test]
    fn test_expand_worktree_template_no_spaces() {
        let template = "../{{repo}}.{{run_branch | sanitize}}";
        let expanded = expand_worktree_template(
            template,
            Path::new("/home/user/my-project"),
            "run/feature-x",
        );
        assert_eq!(expanded, "../my-project.run-feature-x");
    }

    #[test]
    fn test_expand_worktree_template_unsanitized() {
        let template = "/worktrees/{{ run_branch }}";
        let expanded = expand_worktree_template(
            template,
            Path::new("/home/user/my-project"),
            "run/feature-x",
        );
        assert_eq!(expanded, "/worktrees/run/feature-x");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("My Feature"), "my-feature");
        assert_eq!(slugify("add-new-thing"), "add-new-thing");
        assert_eq!(slugify("Fix Bug #123"), "fix-bug-123");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("CamelCase"), "camelcase");
    }

    #[test]
    fn test_normalize_path() {
        let path = normalize_path(Path::new("/home/user/project/../other"));
        assert_eq!(path, PathBuf::from("/home/user/other"));

        let path = normalize_path(Path::new("/home/user/./project"));
        assert_eq!(path, PathBuf::from("/home/user/project"));
    }

    #[test]
    fn test_resolve_worktree_path_absolute() {
        let resolved = resolve_worktree_path("/absolute/path", Path::new("/workspace"));
        assert_eq!(resolved, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_is_working_tree_clean() {
        let dir = setup_test_repo();
        assert!(is_working_tree_clean(dir.path()).unwrap());

        // Create an untracked file.
        std::fs::write(dir.path().join("untracked.txt"), "data").unwrap();
        assert!(!is_working_tree_clean(dir.path()).unwrap());
    }

    #[test]
    fn test_checkout_branch() {
        let dir = setup_test_repo();

        // Create and checkout a new branch.
        create_branch(dir.path(), "feature", "HEAD").unwrap();
        checkout_branch(dir.path(), "feature").unwrap();

        // Verify we're on the new branch.
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let current = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(current, "feature");
    }

    #[test]
    fn test_merge_branch_no_conflict() {
        let dir = setup_test_repo();

        // Create a feature branch with a change.
        create_branch(dir.path(), "feature", "HEAD").unwrap();
        checkout_branch(dir.path(), "feature").unwrap();
        std::fs::write(dir.path().join("feature.txt"), "feature content").unwrap();
        Command::new("git")
            .args(["add", "feature.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Add feature"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Go back to main (or master) and merge.
        let main_branch = detect_default_branch(dir.path()).unwrap();
        checkout_branch(dir.path(), &main_branch).unwrap();
        merge_branch(dir.path(), "feature").unwrap();

        // Verify the file exists after merge.
        assert!(dir.path().join("feature.txt").exists());
    }

    #[test]
    fn test_squash_merge_branch() {
        let dir = setup_test_repo();

        // Create a feature branch with multiple commits.
        create_branch(dir.path(), "feature", "HEAD").unwrap();
        checkout_branch(dir.path(), "feature").unwrap();

        std::fs::write(dir.path().join("file1.txt"), "content1").unwrap();
        Command::new("git")
            .args(["add", "file1.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Add file1"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        std::fs::write(dir.path().join("file2.txt"), "content2").unwrap();
        Command::new("git")
            .args(["add", "file2.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Add file2"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Go back to main and squash merge.
        let main_branch = detect_default_branch(dir.path()).unwrap();
        checkout_branch(dir.path(), &main_branch).unwrap();
        squash_merge_branch(dir.path(), "feature").unwrap();

        // Verify files exist.
        assert!(dir.path().join("file1.txt").exists());
        assert!(dir.path().join("file2.txt").exists());

        // Verify it's a single commit (initial + squash = 2 commits on main).
        let output = Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let count: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_merge_to_target_creates_branch() {
        let dir = setup_test_repo();

        // Create a run branch with changes.
        create_branch(dir.path(), "run/test", "HEAD").unwrap();
        checkout_branch(dir.path(), "run/test").unwrap();
        std::fs::write(dir.path().join("run.txt"), "run content").unwrap();
        Command::new("git")
            .args(["add", "run.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Run changes"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Go back to main for the merge operation.
        let main_branch = detect_default_branch(dir.path()).unwrap();
        checkout_branch(dir.path(), &main_branch).unwrap();

        // Target branch doesn't exist yet; merge_to_target should create it.
        merge_to_target(
            dir.path(),
            "run/test",
            "agent/my-feature",
            &main_branch,
            MergeStrategy::Squash,
        )
        .unwrap();

        // Verify target branch exists and has the file.
        assert!(branch_exists(dir.path(), "agent/my-feature").unwrap());
        assert!(dir.path().join("run.txt").exists());

        // Verify we're on the target branch.
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let current = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(current, "agent/my-feature");
    }

    #[test]
    fn test_merge_to_target_none_strategy_no_op() {
        let dir = setup_test_repo();

        // With MergeStrategy::None, should do nothing.
        let main_branch = detect_default_branch(dir.path()).unwrap();
        merge_to_target(
            dir.path(),
            "nonexistent",
            "target",
            &main_branch,
            MergeStrategy::None,
        )
        .unwrap();

        // Target branch should NOT be created.
        assert!(!branch_exists(dir.path(), "target").unwrap());
    }

    #[test]
    fn test_merge_to_target_dirty_tree_fails() {
        let dir = setup_test_repo();

        // Create uncommitted changes.
        std::fs::write(dir.path().join("dirty.txt"), "uncommitted").unwrap();

        let main_branch = detect_default_branch(dir.path()).unwrap();
        let result = merge_to_target(
            dir.path(),
            "nonexistent",
            "target",
            &main_branch,
            MergeStrategy::Merge,
        );

        assert!(matches!(result, Err(GitError::DirtyWorkingTree(_))));
    }
}
