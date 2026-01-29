//! Worktrunk worktree provider implementation.
//!
//! Uses the `wt` CLI for worktree lifecycle management.
//! See worktrunk-integration.md Section 5.3 (Worktrunk Worktree Creation).

use loop_core::config::Config;
use loop_core::types::{RunWorktree, WorktreeProvider};
use std::path::Path;
use std::process::Command;

use crate::worktree::{Result, WorktreeError, WorktreeProviderTrait};

/// Worktrunk worktree provider using the `wt` CLI.
///
/// See worktrunk-integration.md Section 2.1, 5.3.
pub struct WorktrunkProvider;

impl WorktreeProviderTrait for WorktrunkProvider {
    fn create(&self, workspace_root: &Path, worktree: &RunWorktree, config: &Config) -> Result<()> {
        // Use `wt switch --create <run_branch>` to create the worktree.
        // See spec Section 5.3: Worktrunk Worktree Creation.
        let output = Command::new(&config.worktrunk_bin)
            .args(["switch", "--create", &worktree.run_branch])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| WorktreeError::WorktrunkCommand(format!("failed to execute wt: {}", e)))?;

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
                        "failed to execute wt step copy-ignored: {}",
                        e
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
        // Use `wt remove` to clean up the worktree.
        // See spec Section 5.4: Cleanup.
        //
        // Note: wt remove typically takes the worktree name/branch, not path.
        // We use the run_branch which should match the worktree identifier.
        let output = Command::new(&config.worktrunk_bin)
            .args(["remove", &worktree.run_branch])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| {
                WorktreeError::WorktrunkCommand(format!("failed to execute wt remove: {}", e))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(WorktreeError::WorktrunkCommand(format!(
                "wt remove {} failed: {}",
                worktree.run_branch, stderr
            )));
        }

        Ok(())
    }

    fn provider_type(&self) -> WorktreeProvider {
        WorktreeProvider::Worktrunk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktrunk_provider_type() {
        let provider = WorktrunkProvider;
        assert_eq!(provider.provider_type(), WorktreeProvider::Worktrunk);
    }

    // Integration tests for WorktrunkProvider require the `wt` binary.
    // These are covered by manual QA (spec Section 9).
}
