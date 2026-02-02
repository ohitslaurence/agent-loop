//! Verifier module for executing verification commands.
//!
//! Implements verification execution and failure handling (spec Section 5.2, Section 6.2).
//! Key responsibilities:
//! - Execute `verify_cmds` from config with timeout
//! - Write runner notes on failure with failure context
//! - Signal to scheduler when verification fails (requeue implementation)

use chrono::Utc;
use loop_core::{Config, Step};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum VerifierError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("verification command failed: {cmd}")]
    CommandFailed { cmd: String, exit_code: i32 },
    #[error("verification timeout after {0} seconds")]
    Timeout(u32),
    #[error("no verification commands configured")]
    NoCommands,
}

pub type Result<T> = std::result::Result<T, VerifierError>;

/// Result of verification execution.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Whether all verification commands passed.
    pub passed: bool,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
    /// Individual command results.
    pub commands: Vec<CommandResult>,
    /// Path to runner notes file (only written on failure).
    pub runner_notes_path: Option<PathBuf>,
}

/// Result of a single verification command.
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// The command that was executed.
    pub cmd: String,
    /// Exit code from the command.
    pub exit_code: i32,
    /// Whether this command passed (exit code 0).
    pub passed: bool,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Stdout output.
    pub stdout: String,
    /// Stderr output.
    pub stderr: String,
}

/// Verifier configuration.
#[derive(Debug, Clone, Default)]
pub struct VerifierConfig {
    /// Commands to execute for verification.
    pub verify_cmds: Vec<String>,
    /// Timeout per command in seconds (0 = no timeout).
    pub timeout_sec: u32,
}

impl VerifierConfig {
    /// Create from loop-core Config.
    pub fn from_config(config: &Config) -> Self {
        Self {
            verify_cmds: config.verify_cmds.clone(),
            timeout_sec: config.verify_timeout_sec,
        }
    }
}

/// Verifier for executing verification commands.
#[derive(Debug)]
pub struct Verifier {
    config: VerifierConfig,
}

impl Verifier {
    /// Create a new verifier with the given configuration.
    pub fn new(config: VerifierConfig) -> Self {
        Self { config }
    }

    /// Create a verifier from loop-core Config.
    pub fn from_loop_config(config: &Config) -> Self {
        Self::new(VerifierConfig::from_config(config))
    }

    /// Check if verification is configured.
    pub fn has_commands(&self) -> bool {
        !self.config.verify_cmds.is_empty()
    }

    /// Get the runner notes file path for a run directory.
    ///
    /// Follows bin/loop convention: `<run_dir>/runner-notes.txt`
    pub fn runner_notes_path(run_dir: &Path) -> PathBuf {
        run_dir.join("runner-notes.txt")
    }

    /// Clear runner notes (write empty file).
    pub fn clear_runner_notes(run_dir: &Path) -> Result<()> {
        let path = Self::runner_notes_path(run_dir);
        std::fs::create_dir_all(run_dir)?;
        std::fs::write(&path, "")?;
        Ok(())
    }

    /// Write runner notes with failure context.
    ///
    /// Implements spec Section 5.2: "Verification fails: write runner notes"
    pub fn write_runner_notes(run_dir: &Path, content: &str) -> Result<PathBuf> {
        let path = Self::runner_notes_path(run_dir);
        std::fs::create_dir_all(run_dir)?;
        let mut file = std::fs::File::create(&path)?;
        file.write_all(content.as_bytes())?;
        Ok(path)
    }

    /// Execute all verification commands.
    ///
    /// Returns `VerificationResult` with passed=true if all commands succeed,
    /// or passed=false if any command fails. On failure, writes runner notes.
    pub async fn execute(
        &self,
        step: &Step,
        run_dir: &Path,
        working_dir: &Path,
    ) -> Result<VerificationResult> {
        if !self.has_commands() {
            // No verification configured; treat as pass.
            return Ok(VerificationResult {
                passed: true,
                duration_ms: 0,
                commands: Vec::new(),
                runner_notes_path: None,
            });
        }

        info!(
            step_id = %step.id,
            cmd_count = self.config.verify_cmds.len(),
            "starting verification"
        );

        let start = Utc::now();
        let mut results: Vec<CommandResult> = Vec::new();
        let mut all_passed = true;

        for cmd in &self.config.verify_cmds {
            let result = self.execute_command(cmd, working_dir).await?;

            if !result.passed {
                all_passed = false;
            }
            results.push(result);
            // Continue executing remaining commands to collect all results.
        }

        let end = Utc::now();
        let duration_ms = (end - start).num_milliseconds() as u64;

        let runner_notes_path = if all_passed {
            // Clear runner notes on success.
            Self::clear_runner_notes(run_dir)?;
            None
        } else {
            // Write runner notes with failure context (spec Section 5.2).
            let notes = self.format_failure_notes(&results);
            Some(Self::write_runner_notes(run_dir, &notes)?)
        };

        info!(
            step_id = %step.id,
            passed = all_passed,
            duration_ms = duration_ms,
            "verification complete"
        );

        Ok(VerificationResult {
            passed: all_passed,
            duration_ms,
            commands: results,
            runner_notes_path,
        })
    }

    /// Execute a single verification command.
    async fn execute_command(&self, cmd: &str, working_dir: &Path) -> Result<CommandResult> {
        debug!(cmd = %cmd, "executing verification command");

        let start = Utc::now();

        // Use shell to execute the command (matches bin/loop behavior).
        let mut process = Command::new("sh");
        process
            .arg("-c")
            .arg(cmd)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = process.spawn()?;

        // Take stdout/stderr handles before waiting so we can capture output
        // even if we need to kill the process.
        let mut stdout_handle = child.stdout.take();
        let mut stderr_handle = child.stderr.take();

        // Wait for process with optional timeout.
        let exit_code = if self.config.timeout_sec > 0 {
            let timeout_duration = Duration::from_secs(u64::from(self.config.timeout_sec));

            tokio::select! {
                result = child.wait() => {
                    result?.code().unwrap_or(-1)
                }
                () = tokio::time::sleep(timeout_duration) => {
                    // Kill the process on timeout to prevent zombies
                    if let Err(e) = child.kill().await {
                        warn!(cmd = %cmd, error = %e, "failed to kill timed-out process");
                    }
                    // Reap the process to prevent zombie
                    let _ = child.wait().await;
                    warn!(cmd = %cmd, timeout_sec = self.config.timeout_sec, "verification command timed out");
                    return Err(VerifierError::Timeout(self.config.timeout_sec));
                }
            }
        } else {
            child.wait().await?.code().unwrap_or(-1)
        };

        // Read captured output
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(ref mut handle) = stdout_handle {
            let _ = handle.read_to_end(&mut stdout).await;
        }
        if let Some(ref mut handle) = stderr_handle {
            let _ = handle.read_to_end(&mut stderr).await;
        }

        let end = Utc::now();
        let duration_ms = (end - start).num_milliseconds() as u64;

        let stdout_str = String::from_utf8_lossy(&stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&stderr).to_string();
        let passed = exit_code == 0;

        if passed {
            debug!(cmd = %cmd, duration_ms = duration_ms, "verification command passed");
        } else {
            warn!(cmd = %cmd, exit_code = exit_code, duration_ms = duration_ms, "verification command failed");
        }

        Ok(CommandResult {
            cmd: cmd.to_string(),
            exit_code,
            passed,
            duration_ms,
            stdout: stdout_str,
            stderr: stderr_str,
        })
    }

    /// Format failure notes for runner notes file.
    ///
    /// Follows bin/loop pattern: include failure context that the agent should read.
    fn format_failure_notes(&self, results: &[CommandResult]) -> String {
        let mut notes = String::new();

        notes.push_str("Runner detected failing verification.\n\n");
        notes.push_str(
            "Read this failure context carefully and fix it before doing new plan work:\n\n",
        );

        for result in results {
            if !result.passed {
                notes.push_str(&format!(
                    "--- FAILED: {} (exit {}) ---\n",
                    result.cmd, result.exit_code
                ));

                // Include last 120 lines of output (matches bin/loop).
                let combined_output = if result.stderr.is_empty() {
                    result.stdout.clone()
                } else if result.stdout.is_empty() {
                    result.stderr.clone()
                } else {
                    format!("{}\n\n--- STDERR ---\n{}", result.stdout, result.stderr)
                };

                let lines: Vec<&str> = combined_output.lines().collect();
                let tail_start = lines.len().saturating_sub(120);
                for line in &lines[tail_start..] {
                    notes.push_str(line);
                    notes.push('\n');
                }
                notes.push('\n');
            }
        }

        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{Id, StepPhase, StepStatus};
    use tempfile::TempDir;

    fn create_test_step() -> Step {
        Step {
            id: Id::new(),
            run_id: Id::new(),
            phase: StepPhase::Verification,
            status: StepStatus::InProgress,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        }
    }

    #[test]
    fn verifier_config_from_loop_config() {
        let mut config = Config::default();
        config.verify_cmds = vec!["cargo test".to_string(), "cargo clippy".to_string()];
        config.verify_timeout_sec = 300;

        let verifier_config = VerifierConfig::from_config(&config);
        assert_eq!(verifier_config.verify_cmds.len(), 2);
        assert_eq!(verifier_config.timeout_sec, 300);
    }

    #[test]
    fn runner_notes_path_follows_convention() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        let path = Verifier::runner_notes_path(&run_dir);
        assert_eq!(
            path,
            PathBuf::from("/workspace/logs/loop/run-123/runner-notes.txt")
        );
    }

    #[test]
    fn write_and_clear_runner_notes() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");

        // Write notes.
        let content = "Test failure notes";
        let path = Verifier::write_runner_notes(&run_dir, content).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);

        // Clear notes.
        Verifier::clear_runner_notes(&run_dir).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn has_commands_returns_false_when_empty() {
        let verifier = Verifier::new(VerifierConfig::default());
        assert!(!verifier.has_commands());
    }

    #[test]
    fn has_commands_returns_true_when_configured() {
        let config = VerifierConfig {
            verify_cmds: vec!["cargo test".to_string()],
            timeout_sec: 0,
        };
        let verifier = Verifier::new(config);
        assert!(verifier.has_commands());
    }

    #[tokio::test]
    async fn execute_passes_with_no_commands() {
        let verifier = Verifier::new(VerifierConfig::default());
        let step = create_test_step();
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let working_dir = dir.path();

        let result = verifier
            .execute(&step, &run_dir, working_dir)
            .await
            .unwrap();
        assert!(result.passed);
        assert!(result.commands.is_empty());
    }

    #[tokio::test]
    async fn execute_passes_with_successful_command() {
        let config = VerifierConfig {
            verify_cmds: vec!["true".to_string()],
            timeout_sec: 10,
        };
        let verifier = Verifier::new(config);
        let step = create_test_step();
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let working_dir = dir.path();

        let result = verifier
            .execute(&step, &run_dir, working_dir)
            .await
            .unwrap();
        assert!(result.passed);
        assert_eq!(result.commands.len(), 1);
        assert!(result.commands[0].passed);
        assert_eq!(result.commands[0].exit_code, 0);
    }

    #[tokio::test]
    async fn execute_fails_with_failing_command() {
        let config = VerifierConfig {
            verify_cmds: vec!["false".to_string()],
            timeout_sec: 10,
        };
        let verifier = Verifier::new(config);
        let step = create_test_step();
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let working_dir = dir.path();

        let result = verifier
            .execute(&step, &run_dir, working_dir)
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.commands.len(), 1);
        assert!(!result.commands[0].passed);
        assert!(result.runner_notes_path.is_some());

        // Check runner notes were written.
        let notes_path = result.runner_notes_path.unwrap();
        assert!(notes_path.exists());
        let notes_content = std::fs::read_to_string(&notes_path).unwrap();
        assert!(notes_content.contains("Runner detected failing verification"));
    }

    #[tokio::test]
    async fn execute_continues_after_first_failure() {
        let config = VerifierConfig {
            verify_cmds: vec!["false".to_string(), "true".to_string(), "false".to_string()],
            timeout_sec: 10,
        };
        let verifier = Verifier::new(config);
        let step = create_test_step();
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let working_dir = dir.path();

        let result = verifier
            .execute(&step, &run_dir, working_dir)
            .await
            .unwrap();
        assert!(!result.passed);
        assert_eq!(result.commands.len(), 3);
        assert!(!result.commands[0].passed);
        assert!(result.commands[1].passed);
        assert!(!result.commands[2].passed);
    }

    #[test]
    fn format_failure_notes_includes_context() {
        let verifier = Verifier::new(VerifierConfig::default());
        let results = vec![
            CommandResult {
                cmd: "cargo test".to_string(),
                exit_code: 1,
                passed: false,
                duration_ms: 1000,
                stdout: "test output\nmore output".to_string(),
                stderr: "error output".to_string(),
            },
            CommandResult {
                cmd: "cargo clippy".to_string(),
                exit_code: 0,
                passed: true,
                duration_ms: 500,
                stdout: "".to_string(),
                stderr: "".to_string(),
            },
        ];

        let notes = verifier.format_failure_notes(&results);
        assert!(notes.contains("Runner detected failing verification"));
        assert!(notes.contains("FAILED: cargo test (exit 1)"));
        assert!(notes.contains("test output"));
        assert!(notes.contains("error output"));
        // Should not include passing command.
        assert!(!notes.contains("cargo clippy"));
    }
}
