//! Completion detection for agent output.
//!
//! Matches the behavior of `bin/loop` (Section 5.1, Section 9.1).
//! Two modes are supported:
//! - `Exact`: Output must be exactly `<promise>COMPLETE</promise>` after trimming.
//! - `Trailing`: Last non-empty line must be `<promise>COMPLETE</promise>` after trimming.

use crate::types::CompletionMode;

/// The completion token that signals task completion.
pub const COMPLETION_TOKEN: &str = "<promise>COMPLETE</promise>";

/// Result of completion detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResult {
    /// Whether the output signals completion in the configured mode.
    pub is_complete: bool,
    /// Whether the token was found anywhere in the output (for malformed detection).
    pub token_found: bool,
    /// Whether the token was malformed (found but not accepted).
    pub is_malformed: bool,
}

/// Check if the output signals completion according to the given mode.
///
/// # Arguments
/// * `output` - The raw output from the agent.
/// * `mode` - The completion detection mode to use.
///
/// # Returns
/// A `CompletionResult` indicating completion status and any malformed token detection.
///
/// # Example
/// ```
/// use loop_core::{CompletionMode, completion::{check_completion, COMPLETION_TOKEN}};
///
/// let result = check_completion("<promise>COMPLETE</promise>", CompletionMode::Exact);
/// assert!(result.is_complete);
///
/// let result = check_completion("Done.\n<promise>COMPLETE</promise>", CompletionMode::Trailing);
/// assert!(result.is_complete);
///
/// let result = check_completion("Almost <promise>COMPLETE</promise> done.", CompletionMode::Exact);
/// assert!(!result.is_complete);
/// assert!(result.is_malformed);
/// ```
pub fn check_completion(output: &str, mode: CompletionMode) -> CompletionResult {
    let token_found = output.contains(COMPLETION_TOKEN);

    // Exact mode: entire output (trimmed) must be the token
    let exact_match = {
        let trimmed = output.trim();
        trimmed == COMPLETION_TOKEN
    };

    // Trailing mode: last non-empty line (trimmed) must be the token
    let trailing_match = {
        let last_nonempty_line = output
            .lines().rfind(|line| !line.trim().is_empty())
            .unwrap_or("");
        last_nonempty_line.trim() == COMPLETION_TOKEN
    };

    let is_complete = match mode {
        CompletionMode::Exact => exact_match,
        CompletionMode::Trailing => trailing_match,
    };

    let is_malformed = token_found && !is_complete;

    CompletionResult {
        is_complete,
        token_found,
        is_malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Exact mode tests ---

    #[test]
    fn exact_accepts_bare_token() {
        let result = check_completion(COMPLETION_TOKEN, CompletionMode::Exact);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn exact_accepts_token_with_whitespace() {
        let result = check_completion("  <promise>COMPLETE</promise>\n  ", CompletionMode::Exact);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn exact_rejects_token_with_prefix() {
        let result = check_completion("Done. <promise>COMPLETE</promise>", CompletionMode::Exact);
        assert!(!result.is_complete);
        assert!(result.token_found);
        assert!(result.is_malformed);
    }

    #[test]
    fn exact_rejects_token_with_suffix() {
        let result = check_completion("<promise>COMPLETE</promise> !", CompletionMode::Exact);
        assert!(!result.is_complete);
        assert!(result.token_found);
        assert!(result.is_malformed);
    }

    #[test]
    fn exact_rejects_multiline_output() {
        let output = "Completed task.\n<promise>COMPLETE</promise>";
        let result = check_completion(output, CompletionMode::Exact);
        assert!(!result.is_complete);
        assert!(result.token_found);
        assert!(result.is_malformed);
    }

    // --- Trailing mode tests ---

    #[test]
    fn trailing_accepts_bare_token() {
        let result = check_completion(COMPLETION_TOKEN, CompletionMode::Trailing);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn trailing_accepts_token_as_last_line() {
        let output = "Completed task.\n<promise>COMPLETE</promise>";
        let result = check_completion(output, CompletionMode::Trailing);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn trailing_accepts_token_with_trailing_newlines() {
        let output = "Done.\n<promise>COMPLETE</promise>\n\n";
        let result = check_completion(output, CompletionMode::Trailing);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn trailing_accepts_token_with_whitespace_on_line() {
        let output = "Done.\n  <promise>COMPLETE</promise>  ";
        let result = check_completion(output, CompletionMode::Trailing);
        assert!(result.is_complete);
        assert!(result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn trailing_rejects_token_not_last() {
        let output = "<promise>COMPLETE</promise>\nBut wait, there's more.";
        let result = check_completion(output, CompletionMode::Trailing);
        assert!(!result.is_complete);
        assert!(result.token_found);
        assert!(result.is_malformed);
    }

    #[test]
    fn trailing_rejects_token_embedded_in_line() {
        let output = "Almost <promise>COMPLETE</promise> done.";
        let result = check_completion(output, CompletionMode::Trailing);
        assert!(!result.is_complete);
        assert!(result.token_found);
        assert!(result.is_malformed);
    }

    // --- No token tests ---

    #[test]
    fn no_token_exact() {
        let result = check_completion("Task completed successfully.", CompletionMode::Exact);
        assert!(!result.is_complete);
        assert!(!result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn no_token_trailing() {
        let result = check_completion("Task completed successfully.", CompletionMode::Trailing);
        assert!(!result.is_complete);
        assert!(!result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn empty_output() {
        let result = check_completion("", CompletionMode::Trailing);
        assert!(!result.is_complete);
        assert!(!result.token_found);
        assert!(!result.is_malformed);
    }

    #[test]
    fn whitespace_only_output() {
        let result = check_completion("   \n\n  ", CompletionMode::Trailing);
        assert!(!result.is_complete);
        assert!(!result.token_found);
        assert!(!result.is_malformed);
    }
}
