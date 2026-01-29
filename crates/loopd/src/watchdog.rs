//! Watchdog module for evaluating signals and rewriting prompts.
//!
//! Implements watchdog signal detection and prompt rewrite policy (spec Section 4.2, 5.2, 5.3).
//! Key responsibilities:
//! - Detect watchdog signals (repeated_task, verification_failed, no_progress, malformed_complete)
//! - Evaluate signals and decide on action (rewrite, continue, fail)
//! - Rewrite prompts with audit trail
//! - Cap rewrite attempts per run (default 2)

use loop_core::{WatchdogDecision, WatchdogSignal};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum WatchdogError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("rewrite limit exceeded: {0} attempts")]
    RewriteLimitExceeded(u32),
    #[error("malformed signal data")]
    MalformedSignal,
}

pub type Result<T> = std::result::Result<T, WatchdogError>;

/// Watchdog configuration.
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// Maximum number of prompt rewrites per run (spec Section 5.3: default 2).
    pub max_rewrites: u32,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self { max_rewrites: 2 }
    }
}

/// Watchdog actions after evaluating signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogAction {
    /// Rewrite the prompt and retry the step.
    Rewrite,
    /// Continue without changes.
    Continue,
    /// Fail the run.
    Fail,
}

impl WatchdogAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rewrite => "rewrite",
            Self::Continue => "continue",
            Self::Fail => "fail",
        }
    }
}

/// Information about detected signals for watchdog evaluation.
#[derive(Debug, Clone, Default)]
pub struct SignalContext {
    /// Whether verification has failed.
    pub verification_failed: bool,
    /// Whether the agent is repeating the same task.
    pub repeated_task: bool,
    /// Whether no progress is being made.
    pub no_progress: bool,
    /// Whether the completion token is malformed.
    pub malformed_complete: bool,
    /// Current rewrite count for this run.
    pub current_rewrite_count: u32,
    /// Output from the last step (for analysis).
    pub last_output: Option<String>,
}

impl SignalContext {
    /// Check if any signals are active.
    pub fn has_signals(&self) -> bool {
        self.verification_failed
            || self.repeated_task
            || self.no_progress
            || self.malformed_complete
    }

    /// Get the primary signal (highest priority).
    pub fn primary_signal(&self) -> Option<WatchdogSignal> {
        // Priority order: verification_failed > no_progress > repeated_task > malformed_complete
        if self.verification_failed {
            Some(WatchdogSignal::VerificationFailed)
        } else if self.no_progress {
            Some(WatchdogSignal::NoProgress)
        } else if self.repeated_task {
            Some(WatchdogSignal::RepeatedTask)
        } else if self.malformed_complete {
            Some(WatchdogSignal::MalformedComplete)
        } else {
            None
        }
    }
}

/// Result of prompt rewrite operation.
#[derive(Debug, Clone)]
pub struct RewriteResult {
    /// Path to the original prompt file.
    pub prompt_before: PathBuf,
    /// Path to the rewritten prompt file.
    pub prompt_after: PathBuf,
    /// The rewritten prompt content.
    pub content: String,
}

/// Watchdog for evaluating signals and rewriting prompts.
pub struct Watchdog {
    config: WatchdogConfig,
}

impl Watchdog {
    /// Create a new watchdog with the given configuration.
    pub fn new(config: WatchdogConfig) -> Self {
        Self { config }
    }

    /// Create a watchdog with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(WatchdogConfig::default())
    }

    /// Get the maximum rewrites allowed.
    pub fn max_rewrites(&self) -> u32 {
        self.config.max_rewrites
    }

    /// Evaluate signals and decide on action.
    ///
    /// Implements spec Section 4.2: `evaluate(signals) -> WatchdogDecision`.
    /// Per Section 6: On watchdog error, log and continue without rewrite.
    pub fn evaluate(&self, context: &SignalContext) -> WatchdogDecision {
        let signal = match context.primary_signal() {
            Some(s) => s,
            None => {
                // No signals; continue normally.
                return WatchdogDecision {
                    signal: WatchdogSignal::NoProgress, // Placeholder
                    action: WatchdogAction::Continue.as_str().to_string(),
                    rewrite_count: context.current_rewrite_count,
                    notes: Some("No active signals".to_string()),
                };
            }
        };

        // Check rewrite limit (spec Section 5.3).
        if context.current_rewrite_count >= self.config.max_rewrites {
            info!(
                rewrite_count = context.current_rewrite_count,
                max_rewrites = self.config.max_rewrites,
                "rewrite limit reached, failing run"
            );
            return WatchdogDecision {
                signal,
                action: WatchdogAction::Fail.as_str().to_string(),
                rewrite_count: context.current_rewrite_count,
                notes: Some(format!(
                    "Rewrite limit exceeded ({} of {})",
                    context.current_rewrite_count, self.config.max_rewrites
                )),
            };
        }

        // Decide action based on signal type.
        let action = match signal {
            WatchdogSignal::VerificationFailed => {
                // Verification failure is handled by scheduler requeuing implementation.
                // Watchdog doesn't rewrite for this; it's handled via runner notes.
                WatchdogAction::Continue
            }
            WatchdogSignal::NoProgress | WatchdogSignal::RepeatedTask => {
                // These signals trigger prompt rewrite.
                WatchdogAction::Rewrite
            }
            WatchdogSignal::MalformedComplete => {
                // Malformed completion might indicate a bug; rewrite to clarify.
                WatchdogAction::Rewrite
            }
        };

        let notes = match action {
            WatchdogAction::Rewrite => Some(format!("Rewriting prompt due to {:?}", signal)),
            WatchdogAction::Continue => Some(format!("Continuing despite {:?}", signal)),
            WatchdogAction::Fail => Some(format!("Failing run due to {:?}", signal)),
        };

        info!(
            signal = ?signal,
            action = ?action,
            rewrite_count = context.current_rewrite_count,
            "watchdog decision"
        );

        WatchdogDecision {
            signal,
            action: action.as_str().to_string(),
            rewrite_count: context.current_rewrite_count,
            notes,
        }
    }

    /// Rewrite the prompt based on the detected signal.
    ///
    /// Per spec Section 4.3, the rewritten prompt is saved to `prompt.rewrite.N.txt`.
    /// Per Section 5.2, "Watchdog rewrites prompt: re-run same phase with incremented attempt."
    pub fn rewrite_prompt(
        &self,
        run_dir: &Path,
        original_prompt: &str,
        signal: WatchdogSignal,
        rewrite_count: u32,
    ) -> Result<RewriteResult> {
        let prompt_before = run_dir.join("prompt.txt");
        let prompt_after = run_dir.join(format!("prompt.rewrite.{}.txt", rewrite_count + 1));

        let rewrite_instruction = self.get_rewrite_instruction(signal);
        let rewritten_content = format!(
            "{}\n\n---\n\n## Watchdog Intervention\n\n{}\n\n---\n\n{}",
            "# IMPORTANT: Read this section carefully before continuing",
            rewrite_instruction,
            original_prompt
        );

        // Write the rewritten prompt.
        std::fs::create_dir_all(run_dir)?;
        let mut file = std::fs::File::create(&prompt_after)?;
        file.write_all(rewritten_content.as_bytes())?;

        debug!(
            prompt_before = %prompt_before.display(),
            prompt_after = %prompt_after.display(),
            signal = ?signal,
            "prompt rewritten"
        );

        Ok(RewriteResult {
            prompt_before,
            prompt_after,
            content: rewritten_content,
        })
    }

    /// Get the rewrite instruction for a given signal.
    fn get_rewrite_instruction(&self, signal: WatchdogSignal) -> &'static str {
        match signal {
            WatchdogSignal::VerificationFailed => {
                "Verification has failed. Review the runner notes file for failure details. \
                 Fix the issues before continuing with new plan work."
            }
            WatchdogSignal::RepeatedTask => {
                "The watchdog detected that you are repeating the same task multiple times. \
                 This suggests you may be stuck in a loop. Please:\n\
                 1. Review what you've already done\n\
                 2. Identify why the task isn't completing\n\
                 3. Try a different approach or ask for clarification"
            }
            WatchdogSignal::NoProgress => {
                "The watchdog detected no meaningful progress on the task. Please:\n\
                 1. Verify you understand the requirements correctly\n\
                 2. Break down the task into smaller, concrete steps\n\
                 3. Complete at least one step before the next iteration"
            }
            WatchdogSignal::MalformedComplete => {
                "The completion token was malformed. When the task is complete, \
                 output exactly: <promise>COMPLETE</promise>\n\
                 Ensure it is on its own line with no surrounding whitespace or text."
            }
        }
    }

    /// Generate the rewritten prompt path for a given rewrite number.
    pub fn rewrite_path(run_dir: &Path, rewrite_number: u32) -> PathBuf {
        run_dir.join(format!("prompt.rewrite.{}.txt", rewrite_number))
    }

    /// Detect signals from step output and run state.
    ///
    /// This is a heuristic-based signal detector.
    pub fn detect_signals(
        &self,
        output: &str,
        previous_outputs: &[String],
        verification_failed: bool,
    ) -> SignalContext {
        let mut context = SignalContext {
            verification_failed,
            current_rewrite_count: 0,
            last_output: Some(output.to_string()),
            ..Default::default()
        };

        // Detect repeated task by comparing with previous outputs.
        if !previous_outputs.is_empty() {
            let similarity = self.compute_output_similarity(output, previous_outputs);
            if similarity > 0.85 {
                context.repeated_task = true;
                debug!(similarity = similarity, "detected repeated task");
            }
        }

        // Detect no progress by looking for stall patterns.
        if self.detect_stall_patterns(output) {
            context.no_progress = true;
            debug!("detected no progress");
        }

        // Detect malformed completion token.
        if self.detect_malformed_complete(output) {
            context.malformed_complete = true;
            debug!("detected malformed completion token");
        }

        context
    }

    /// Compute similarity between current output and previous outputs.
    /// Returns a value between 0.0 (completely different) and 1.0 (identical).
    fn compute_output_similarity(&self, current: &str, previous: &[String]) -> f64 {
        if previous.is_empty() {
            return 0.0;
        }

        // Simple similarity: compare last output.
        let last = &previous[previous.len() - 1];

        // Normalize outputs for comparison.
        let current_lines: Vec<&str> = current.lines().collect();
        let last_lines: Vec<&str> = last.lines().collect();

        if current_lines.is_empty() && last_lines.is_empty() {
            return 1.0;
        }

        // Count matching lines.
        let mut matches = 0;
        let total = current_lines.len().max(last_lines.len());

        for (i, line) in current_lines.iter().enumerate() {
            if i < last_lines.len() && line == &last_lines[i] {
                matches += 1;
            }
        }

        matches as f64 / total as f64
    }

    /// Detect patterns indicating the agent is stalled.
    fn detect_stall_patterns(&self, output: &str) -> bool {
        let stall_indicators = [
            "I don't have enough information",
            "I cannot proceed",
            "I'm stuck",
            "I'm unable to",
            "I cannot determine",
            "waiting for clarification",
            "need more context",
        ];

        let output_lower = output.to_lowercase();
        for indicator in stall_indicators {
            if output_lower.contains(&indicator.to_lowercase()) {
                return true;
            }
        }

        false
    }

    /// Detect if the output contains a malformed completion token.
    fn detect_malformed_complete(&self, output: &str) -> bool {
        // Look for partial matches that aren't the exact token.
        let has_promise = output.contains("<promise>") || output.contains("</promise>");
        let has_complete = output.to_lowercase().contains("complete");
        let has_exact_token = output.contains("<promise>COMPLETE</promise>");

        // Malformed if it has parts but not the exact token.
        if (has_promise || has_complete) && !has_exact_token {
            // Check for common malformations.
            let malformed_patterns = [
                "<promise> COMPLETE </promise>",
                "<promise>complete</promise>",
                "<promise>COMPLETE</promise >",
            ];
            for pattern in malformed_patterns {
                if output.contains(pattern) {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn watchdog_config_defaults() {
        let config = WatchdogConfig::default();
        assert_eq!(config.max_rewrites, 2);
    }

    #[test]
    fn signal_context_detects_no_signals() {
        let context = SignalContext::default();
        assert!(!context.has_signals());
        assert!(context.primary_signal().is_none());
    }

    #[test]
    fn signal_context_priority_order() {
        // verification_failed has highest priority.
        let context = SignalContext {
            verification_failed: true,
            no_progress: true,
            repeated_task: true,
            ..Default::default()
        };
        assert_eq!(
            context.primary_signal(),
            Some(WatchdogSignal::VerificationFailed)
        );

        // no_progress is second.
        let context = SignalContext {
            no_progress: true,
            repeated_task: true,
            ..Default::default()
        };
        assert_eq!(context.primary_signal(), Some(WatchdogSignal::NoProgress));
    }

    #[test]
    fn evaluate_returns_continue_with_no_signals() {
        let watchdog = Watchdog::with_defaults();
        let context = SignalContext::default();
        let decision = watchdog.evaluate(&context);
        assert_eq!(decision.action, "continue");
    }

    #[test]
    fn evaluate_returns_rewrite_for_no_progress() {
        let watchdog = Watchdog::with_defaults();
        let context = SignalContext {
            no_progress: true,
            ..Default::default()
        };
        let decision = watchdog.evaluate(&context);
        assert_eq!(decision.action, "rewrite");
        assert_eq!(decision.signal, WatchdogSignal::NoProgress);
    }

    #[test]
    fn evaluate_returns_fail_when_limit_exceeded() {
        let watchdog = Watchdog::with_defaults();
        let context = SignalContext {
            no_progress: true,
            current_rewrite_count: 2, // At limit.
            ..Default::default()
        };
        let decision = watchdog.evaluate(&context);
        assert_eq!(decision.action, "fail");
    }

    #[test]
    fn evaluate_continues_for_verification_failed() {
        // Verification failure is handled by scheduler, not watchdog rewrite.
        let watchdog = Watchdog::with_defaults();
        let context = SignalContext {
            verification_failed: true,
            ..Default::default()
        };
        let decision = watchdog.evaluate(&context);
        assert_eq!(decision.action, "continue");
    }

    #[test]
    fn rewrite_prompt_creates_file() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let watchdog = Watchdog::with_defaults();

        let result = watchdog
            .rewrite_prompt(&run_dir, "Original prompt", WatchdogSignal::NoProgress, 0)
            .unwrap();

        assert!(result.prompt_after.exists());
        assert_eq!(result.prompt_after, run_dir.join("prompt.rewrite.1.txt"));

        let content = std::fs::read_to_string(&result.prompt_after).unwrap();
        assert!(content.contains("Original prompt"));
        assert!(content.contains("Watchdog Intervention"));
        assert!(content.contains("no meaningful progress"));
    }

    #[test]
    fn rewrite_path_generates_correct_name() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        assert_eq!(
            Watchdog::rewrite_path(&run_dir, 1),
            PathBuf::from("/workspace/logs/loop/run-123/prompt.rewrite.1.txt")
        );
        assert_eq!(
            Watchdog::rewrite_path(&run_dir, 3),
            PathBuf::from("/workspace/logs/loop/run-123/prompt.rewrite.3.txt")
        );
    }

    #[test]
    fn detect_signals_identifies_repeated_task() {
        let watchdog = Watchdog::with_defaults();
        let output = "Line 1\nLine 2\nLine 3";
        let previous = vec!["Line 1\nLine 2\nLine 3".to_string()];

        let context = watchdog.detect_signals(output, &previous, false);
        assert!(context.repeated_task);
    }

    #[test]
    fn detect_signals_identifies_stall() {
        let watchdog = Watchdog::with_defaults();
        let output = "I don't have enough information to proceed with this task.";
        let previous: Vec<String> = vec![];

        let context = watchdog.detect_signals(output, &previous, false);
        assert!(context.no_progress);
    }

    #[test]
    fn detect_malformed_complete_identifies_issues() {
        let watchdog = Watchdog::with_defaults();

        // Correct token - not malformed.
        assert!(!watchdog.detect_malformed_complete("<promise>COMPLETE</promise>"));

        // Malformed tokens.
        assert!(watchdog.detect_malformed_complete("<promise> COMPLETE </promise>"));
        assert!(watchdog.detect_malformed_complete("<promise>complete</promise>"));
    }

    #[test]
    fn compute_similarity_identical_outputs() {
        let watchdog = Watchdog::with_defaults();
        let current = "Line 1\nLine 2";
        let previous = vec!["Line 1\nLine 2".to_string()];
        let similarity = watchdog.compute_output_similarity(current, &previous);
        assert!((similarity - 1.0).abs() < 0.01);
    }

    #[test]
    fn compute_similarity_different_outputs() {
        let watchdog = Watchdog::with_defaults();
        let current = "Completely different\nNew lines";
        let previous = vec!["Old content\nOther stuff".to_string()];
        let similarity = watchdog.compute_output_similarity(current, &previous);
        assert!(similarity < 0.5);
    }

    #[test]
    fn watchdog_action_as_str() {
        assert_eq!(WatchdogAction::Rewrite.as_str(), "rewrite");
        assert_eq!(WatchdogAction::Continue.as_str(), "continue");
        assert_eq!(WatchdogAction::Fail.as_str(), "fail");
    }
}
