//! Worktree provider interface and implementations.
//!
//! Provides an abstraction for worktree lifecycle management.
//! See worktrunk-integration.md Section 2.1, 4.2.
//!
//! Implementations:
//! - Git provider: wraps `crates/loopd/src/git.rs` for native git worktree operations.
//! - Worktrunk provider: uses `wt` CLI (in `worktree_worktrunk.rs`).

use loop_core::config::Config;
use loop_core::types::{RunWorktree, WorktreeProvider};
use std::path::Path;
use std::process::Command;
use thiserror::Error;

use crate::git;
use crate::worktree_worktrunk::WorktrunkProvider;

#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("git error: {0}")]
    Git(#[from] git::GitError),
    #[error("provider not available: {0}")]
    ProviderNotAvailable(String),
    #[error("worktree path resolution failed: {0}")]
    PathResolution(String),
    #[error("worktrunk command failed: {0}")]
    WorktrunkCommand(String),
}

pub type Result<T> = std::result::Result<T, WorktreeError>;

/// Trait for worktree lifecycle management.
///
/// See worktrunk-integration.md Section 4.2 (Internal APIs).
pub trait WorktreeProviderTrait: Send + Sync {
    /// Create a worktree for a run.
    ///
    /// Creates the worktree at `worktree.worktree_path` with the run branch
    /// checked out, creating the branch from base if needed.
    fn create(&self, workspace_root: &Path, worktree: &RunWorktree, config: &Config) -> Result<()>;

    /// Remove a worktree after run completion.
    ///
    /// Called when `worktree_cleanup=true`. Failures are logged but do not
    /// fail completed runs (Section 6.2).
    fn cleanup(&self, workspace_root: &Path, worktree: &RunWorktree, config: &Config)
        -> Result<()>;

    /// Get the provider type.
    fn provider_type(&self) -> WorktreeProvider;
}

/// Resolve the effective worktree provider based on config and availability.
///
/// See worktrunk-integration.md Section 5.2 (Provider Selection):
/// - `auto`: use Worktrunk if `wt` is available, else fallback to git.
/// - `worktrunk`: fail if `wt` is not available.
/// - `git`: always use native git.
pub fn resolve_provider(config: &Config, _workspace_root: &Path) -> Result<WorktreeProvider> {
    match config.worktree_provider {
        WorktreeProvider::Auto => {
            if is_worktrunk_available(&config.worktrunk_bin) {
                Ok(WorktreeProvider::Worktrunk)
            } else {
                Ok(WorktreeProvider::Git)
            }
        }
        WorktreeProvider::Worktrunk => {
            if is_worktrunk_available(&config.worktrunk_bin) {
                Ok(WorktreeProvider::Worktrunk)
            } else {
                Err(WorktreeError::ProviderNotAvailable(
                    "worktrunk requested but `wt` is not available".to_string(),
                ))
            }
        }
        WorktreeProvider::Git => Ok(WorktreeProvider::Git),
    }
}

/// Check if the Worktrunk CLI (`wt`) is available.
fn is_worktrunk_available(worktrunk_bin: &Path) -> bool {
    Command::new(worktrunk_bin)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Prepare a worktree for a run using the resolved provider.
///
/// This is the main entry point for worktree creation. It:
/// 1. Uses the provider from `worktree.provider` (already resolved)
/// 2. Delegates to the appropriate provider implementation
///
/// See worktrunk-integration.md Section 4.2 (Internal APIs).
pub fn prepare(workspace_root: &Path, worktree: &RunWorktree, config: &Config) -> Result<()> {
    let provider = get_provider(worktree.provider);
    provider.create(workspace_root, worktree, config)
}

/// Clean up a worktree after run completion.
///
/// See worktrunk-integration.md Section 5.4:
/// - If `worktree_cleanup=true` (default false), removes the worktree.
/// - Cleanup failures are logged but do not fail completed runs.
pub fn cleanup(workspace_root: &Path, worktree: &RunWorktree, config: &Config) -> Result<()> {
    let provider = get_provider(worktree.provider);
    provider.cleanup(workspace_root, worktree, config)
}

/// Get a provider implementation for the given provider type.
fn get_provider(provider: WorktreeProvider) -> Box<dyn WorktreeProviderTrait> {
    match provider {
        WorktreeProvider::Git | WorktreeProvider::Auto => Box::new(GitProvider),
        WorktreeProvider::Worktrunk => Box::new(WorktrunkProvider),
    }
}

/// Git worktree provider using native git commands.
///
/// Wraps functions from `crates/loopd/src/git.rs`.
/// See worktrunk-integration.md Section 2.1.
pub struct GitProvider;

impl WorktreeProviderTrait for GitProvider {
    fn create(
        &self,
        workspace_root: &Path,
        worktree: &RunWorktree,
        _config: &Config,
    ) -> Result<()> {
        let worktree_path = Path::new(&worktree.worktree_path);
        git::create_worktree(
            workspace_root,
            worktree_path,
            &worktree.run_branch,
            &worktree.base_branch,
        )?;
        Ok(())
    }

    fn cleanup(
        &self,
        workspace_root: &Path,
        worktree: &RunWorktree,
        _config: &Config,
    ) -> Result<()> {
        let worktree_path = Path::new(&worktree.worktree_path);
        git::remove_worktree(workspace_root, worktree_path)?;
        Ok(())
    }

    fn provider_type(&self) -> WorktreeProvider {
        WorktreeProvider::Git
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::types::MergeStrategy;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    fn setup_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        StdCommand::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        StdCommand::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn resolve_provider_git_always_works() {
        let mut config = Config::default();
        config.worktree_provider = WorktreeProvider::Git;
        let result = resolve_provider(&config, Path::new("/tmp"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), WorktreeProvider::Git);
    }

    #[test]
    fn resolve_provider_auto_falls_back_to_git() {
        let mut config = Config::default();
        config.worktree_provider = WorktreeProvider::Auto;
        // With a non-existent binary, should fall back to git
        config.worktrunk_bin = PathBuf::from("/nonexistent/wt");
        let result = resolve_provider(&config, Path::new("/tmp"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), WorktreeProvider::Git);
    }

    #[test]
    fn resolve_provider_worktrunk_fails_if_missing() {
        let mut config = Config::default();
        config.worktree_provider = WorktreeProvider::Worktrunk;
        config.worktrunk_bin = PathBuf::from("/nonexistent/wt");
        let result = resolve_provider(&config, Path::new("/tmp"));
        assert!(result.is_err());
        assert!(matches!(
            result,
            Err(WorktreeError::ProviderNotAvailable(_))
        ));
    }

    #[test]
    fn git_provider_create_and_cleanup() {
        let dir = setup_test_repo();
        let config = Config::default();

        // Detect default branch (may be main or master)
        let base_branch = git::detect_default_branch(dir.path()).unwrap_or("main".to_string());

        // Set up worktree config
        let worktree_path = dir.path().parent().unwrap().join("test-worktree");
        let worktree = RunWorktree {
            base_branch,
            run_branch: "run/test".to_string(),
            merge_target_branch: None,
            merge_strategy: MergeStrategy::None,
            worktree_path: worktree_path.to_string_lossy().to_string(),
            provider: WorktreeProvider::Git,
        };

        let provider = GitProvider;

        // Create worktree
        let result = provider.create(dir.path(), &worktree, &config);
        assert!(result.is_ok(), "create failed: {:?}", result);
        assert!(worktree_path.exists());

        // Verify branch exists
        let output = StdCommand::new("git")
            .args(["branch", "--list", "run/test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let branches = String::from_utf8_lossy(&output.stdout);
        assert!(branches.contains("run/test"));

        // Cleanup worktree
        let result = provider.cleanup(dir.path(), &worktree, &config);
        assert!(result.is_ok(), "cleanup failed: {:?}", result);
        assert!(!worktree_path.exists());
    }

    #[test]
    fn git_provider_type() {
        let provider = GitProvider;
        assert_eq!(provider.provider_type(), WorktreeProvider::Git);
    }

    // --- Integration tests for worktree::prepare/cleanup (Section 6.2) ---

    #[test]
    fn prepare_and_cleanup_with_git_provider() {
        // Integration test: verify prepare() and cleanup() work end-to-end
        // with the resolved git provider (spec Section 6.2).
        let dir = setup_test_repo();
        let config = Config::default();

        let base_branch = git::detect_default_branch(dir.path()).unwrap_or("main".to_string());
        let worktree_path = dir.path().parent().unwrap().join("integration-worktree");

        let worktree = RunWorktree {
            base_branch,
            run_branch: "run/integration".to_string(),
            merge_target_branch: None,
            merge_strategy: MergeStrategy::None,
            worktree_path: worktree_path.to_string_lossy().to_string(),
            provider: WorktreeProvider::Git,
        };

        // Use module-level prepare function (tests get_provider routing)
        let result = prepare(dir.path(), &worktree, &config);
        assert!(result.is_ok(), "prepare failed: {:?}", result);
        assert!(worktree_path.exists(), "worktree not created");

        // Use module-level cleanup function
        let result = cleanup(dir.path(), &worktree, &config);
        assert!(result.is_ok(), "cleanup failed: {:?}", result);
        assert!(!worktree_path.exists(), "worktree not removed");
    }

    #[test]
    fn resolve_provider_auto_with_missing_wt_falls_back() {
        // Integration test: auto provider falls back to git when wt missing.
        // Spec Section 6.2: "Auto provider: fallback to git when Worktrunk is missing."
        let mut config = Config::default();
        config.worktree_provider = WorktreeProvider::Auto;
        config.worktrunk_bin = PathBuf::from("/nonexistent/path/to/wt");

        let result = resolve_provider(&config, Path::new("/any/path"));
        assert!(result.is_ok());
        // Verify we get git, not an error
        assert_eq!(result.unwrap(), WorktreeProvider::Git);
    }

    #[test]
    fn resolve_provider_worktrunk_explicit_fails_when_missing() {
        // Integration test: explicit worktrunk provider fails if wt is missing.
        // Spec Section 6.2: "Hard provider: mark run FAILED with reason."
        let mut config = Config::default();
        config.worktree_provider = WorktreeProvider::Worktrunk;
        config.worktrunk_bin = PathBuf::from("/nonexistent/path/to/wt");

        let result = resolve_provider(&config, Path::new("/any/path"));
        assert!(result.is_err());

        let err = result.unwrap_err();
        // Verify error type and message content
        assert!(matches!(err, WorktreeError::ProviderNotAvailable(_)));
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("worktrunk"),
            "error should mention worktrunk: {}",
            err_msg
        );
    }

    #[test]
    fn get_provider_routes_correctly() {
        // Verify get_provider returns correct provider implementations.
        let git_provider = get_provider(WorktreeProvider::Git);
        assert_eq!(git_provider.provider_type(), WorktreeProvider::Git);

        let worktrunk_provider = get_provider(WorktreeProvider::Worktrunk);
        assert_eq!(
            worktrunk_provider.provider_type(),
            WorktreeProvider::Worktrunk
        );

        // Auto should route to Git (as the default/fallback)
        let auto_provider = get_provider(WorktreeProvider::Auto);
        assert_eq!(auto_provider.provider_type(), WorktreeProvider::Git);
    }
}
