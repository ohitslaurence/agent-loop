//! Prompt assembly for the orchestrator daemon.
//!
//! This module will handle prompt generation to match `bin/loop` behavior.
//! Implementation deferred to Phase 2 (runner).

use crate::types::CompletionMode;
use std::path::Path;

/// Completion token that agents emit when done.
pub const COMPLETION_TOKEN: &str = "<promise>COMPLETE</promise>";

/// Check if output indicates completion based on the mode.
pub fn is_complete(output: &str, mode: CompletionMode) -> bool {
    let trimmed = output.trim();

    match mode {
        CompletionMode::Exact => trimmed == COMPLETION_TOKEN,
        CompletionMode::Trailing => {
            // Last non-empty line must be the token
            trimmed
                .lines()
                .filter(|line| !line.trim().is_empty())
                .last()
                .map(|line| line.trim() == COMPLETION_TOKEN)
                .unwrap_or(false)
        }
    }
}

/// Generate a slug from a spec path for naming purposes.
pub fn spec_slug(spec_path: &Path) -> String {
    spec_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            s.chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .to_lowercase()
        })
        .unwrap_or_else(|| "unnamed".to_string())
}

/// Sanitize a branch name for filesystem use (replace slashes with dashes).
pub fn sanitize_branch_name(branch: &str) -> String {
    branch.replace('/', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_complete_exact_mode() {
        assert!(is_complete(
            "<promise>COMPLETE</promise>",
            CompletionMode::Exact
        ));
        assert!(is_complete(
            "  <promise>COMPLETE</promise>  ",
            CompletionMode::Exact
        ));
        assert!(!is_complete(
            "Some output\n<promise>COMPLETE</promise>",
            CompletionMode::Exact
        ));
    }

    #[test]
    fn is_complete_trailing_mode() {
        assert!(is_complete(
            "<promise>COMPLETE</promise>",
            CompletionMode::Trailing
        ));
        assert!(is_complete(
            "Completed task. 0 tasks remain.\n<promise>COMPLETE</promise>",
            CompletionMode::Trailing
        ));
        assert!(is_complete(
            "Some output\n<promise>COMPLETE</promise>\n\n",
            CompletionMode::Trailing
        ));
        assert!(!is_complete(
            "<promise>COMPLETE</promise>\nMore output",
            CompletionMode::Trailing
        ));
    }

    #[test]
    fn spec_slug_extracts_name() {
        assert_eq!(spec_slug(Path::new("specs/my-feature.md")), "my-feature");
        assert_eq!(
            spec_slug(Path::new("/path/to/orchestrator-daemon.md")),
            "orchestrator-daemon"
        );
        assert_eq!(spec_slug(Path::new("no_extension")), "no-extension");
    }

    #[test]
    fn sanitize_branch_name_replaces_slashes() {
        assert_eq!(sanitize_branch_name("run/my-feature"), "run-my-feature");
        assert_eq!(sanitize_branch_name("feature/sub/deep"), "feature-sub-deep");
        assert_eq!(sanitize_branch_name("no-slashes"), "no-slashes");
    }
}
