//! Worktrunk worktree provider implementation.
//!
//! Uses the `wt` CLI for worktree lifecycle management.
//! See worktrunk-integration.md Section 5.3 (Worktrunk Worktree Creation).

use loop_core::config::Config;
use loop_core::types::{RunWorktree, WorktreeProvider};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::git;
use crate::worktree::{Result, WorktreeError, WorktreeProviderTrait};
use tracing::warn;

/// Worktrunk configuration (subset we care about).
///
/// See worktrunk-integration.md Section 5.3:
/// - Worktree path derived from Worktrunk config `worktree-path`.
#[derive(Debug, Deserialize)]
struct WorktrunkConfig {
    #[serde(rename = "worktree-path")]
    worktree_path: Option<String>,
}

/// Resolve the worktree-path template from Worktrunk config.
///
/// Returns the template from the config file if available, otherwise None.
/// Per spec Section 5.3, callers should fall back to the default template.
///
/// See worktrunk-integration.md Section 8 (Data Handling):
/// - Do not log config contents beyond the worktree-path template.
pub fn resolve_worktree_path_template(config: &Config) -> Option<String> {
    let config_path = resolve_config_path(config)?;

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                "could not read Worktrunk config at {}: {}",
                config_path.display(),
                e
            );
            return None;
        }
    };

    let parsed: WorktrunkConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                "could not parse Worktrunk config at {}: {}",
                config_path.display(),
                e
            );
            return None;
        }
    };

    parsed.worktree_path
}

/// Resolve the Worktrunk config file path.
///
/// Uses `config.worktrunk_config_path` if set, otherwise defaults to
/// ~/.config/worktrunk/config.toml (per spec Section 4.1).
fn resolve_config_path(config: &Config) -> Option<PathBuf> {
    if let Some(ref path) = config.worktrunk_config_path {
        // Expand ~ if present.
        let expanded = expand_tilde(path);
        return Some(expanded);
    }

    // Default: ~/.config/worktrunk/config.toml
    dirs::config_dir().map(|d| d.join("worktrunk").join("config.toml"))
}

/// Expand ~ to home directory in a path.
fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path_str[2..]);
        }
    }
    path.to_path_buf()
}

/// Worktrunk worktree provider using the `wt` CLI.
///
/// See worktrunk-integration.md Section 2.1, 5.3.
#[derive(Debug)]
pub struct WorktrunkProvider;

impl WorktreeProviderTrait for WorktrunkProvider {
    fn create(&self, workspace_root: &Path, worktree: &RunWorktree, config: &Config) -> Result<()> {
        // Use `wt switch --create <run_branch> --base <base_branch>` to create the worktree.
        // See spec Section 5.3: Worktrunk Worktree Creation.
        // Must pass --base to ensure the worktree is based on the correct branch,
        // otherwise wt defaults to the repo's default branch.
        let output = Command::new(&config.worktrunk_bin)
            .args([
                "switch",
                "--create",
                &worktree.run_branch,
                "--base",
                &worktree.base_branch,
            ])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| WorktreeError::WorktrunkCommand(format!("failed to execute wt: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorktreeError::WorktrunkCommand(format!(
                "wt switch --create {} failed: {}",
                worktree.run_branch, stderr
            )));
        }

        // Optional: copy ignored files (spec Section 5.3).
        if config.worktrunk_copy_ignored {
            let copy_output = Command::new(&config.worktrunk_bin)
                .args(["step", "copy-ignored"])
                .current_dir(workspace_root)
                .output()
                .map_err(|e| {
                    WorktreeError::WorktrunkCommand(format!(
                        "failed to execute wt step copy-ignored: {e}"
                    ))
                })?;

            if !copy_output.status.success() {
                let stderr = String::from_utf8_lossy(&copy_output.stderr);
                // Log but don't fail - copy-ignored is optional.
                tracing::warn!("wt step copy-ignored failed (non-fatal): {}", stderr.trim());
            }
        }

        Ok(())
    }

    fn cleanup(
        &self,
        workspace_root: &Path,
        worktree: &RunWorktree,
        config: &Config,
    ) -> Result<()> {
        let run_branch = &worktree.run_branch;
        // Capture the run branch SHA before removal in case wt deletes it.
        let branch_sha = resolve_branch_sha(workspace_root, run_branch);

        // Use `wt remove` to clean up the worktree.
        // See spec Section 5.4: Cleanup.
        //
        // Note: wt remove typically takes the worktree name/branch, not path.
        // We use the run_branch which should match the worktree identifier.
        let output = Command::new(&config.worktrunk_bin)
            .args(["remove", run_branch])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| {
                WorktreeError::WorktrunkCommand(format!("failed to execute wt remove: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorktreeError::WorktrunkCommand(format!(
                "wt remove {} failed: {}",
                run_branch, stderr
            )));
        }

        if let Some(sha) = branch_sha {
            if !git::branch_exists(workspace_root, run_branch)? {
                git::create_branch(workspace_root, run_branch, &sha)?;
                warn!(
                    run_branch = %run_branch,
                    "restored run branch after worktrunk cleanup"
                );
            }
        }

        Ok(())
    }

    fn provider_type(&self) -> WorktreeProvider {
        WorktreeProvider::Worktrunk
    }
}

fn resolve_branch_sha(workspace_root: &Path, branch: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", &format!("refs/heads/{branch}")])
        .current_dir(workspace_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn worktrunk_provider_type() {
        let provider = WorktrunkProvider;
        assert_eq!(provider.provider_type(), WorktreeProvider::Worktrunk);
    }

    #[test]
    fn parse_worktrunk_config_with_worktree_path() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
worktree-path = "/custom/worktrees/{{ repo }}.{{ branch }}"
some-other-key = "ignored"
"#,
        )
        .unwrap();

        let mut config = Config::default();
        config.worktrunk_config_path = Some(config_path);

        let result = resolve_worktree_path_template(&config);
        assert_eq!(
            result,
            Some("/custom/worktrees/{{ repo }}.{{ branch }}".to_string())
        );
    }

    #[test]
    fn parse_worktrunk_config_without_worktree_path() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
other-setting = "value"
"#,
        )
        .unwrap();

        let mut config = Config::default();
        config.worktrunk_config_path = Some(config_path);

        let result = resolve_worktree_path_template(&config);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_worktrunk_config_missing_file() {
        let mut config = Config::default();
        config.worktrunk_config_path = Some(PathBuf::from("/nonexistent/config.toml"));

        let result = resolve_worktree_path_template(&config);
        assert_eq!(result, None);
    }

    #[test]
    fn parse_worktrunk_config_invalid_toml() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "not valid toml {{{{").unwrap();

        let mut config = Config::default();
        config.worktrunk_config_path = Some(config_path);

        let result = resolve_worktree_path_template(&config);
        assert_eq!(result, None);
    }

    #[test]
    fn expand_tilde_expands_home() {
        let path = Path::new("~/some/path");
        let expanded = expand_tilde(path);
        // Should not contain ~ anymore (unless HOME is not set).
        if dirs::home_dir().is_some() {
            assert!(!expanded.to_string_lossy().starts_with("~/"));
            assert!(expanded.to_string_lossy().ends_with("some/path"));
        }
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths() {
        let path = Path::new("/absolute/path");
        let expanded = expand_tilde(path);
        assert_eq!(expanded, PathBuf::from("/absolute/path"));
    }

    // Integration tests for WorktrunkProvider require the `wt` binary.
    // These are covered by manual QA (spec Section 9).
}
