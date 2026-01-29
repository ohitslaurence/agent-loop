//! Runner module for executing Claude CLI with retries and timeouts.
//!
//! Implements step execution via the Claude CLI (spec Section 4.2, 5.3, 7.1).
//! Key responsibilities:
//! - Execute Claude CLI with configurable timeout
//! - Retry on failure with exponential backoff
//! - Write artifacts (iter-XX.log, iter-XX.tail.txt)
//! - Track step timing and exit codes

use chrono::Utc;
use loop_core::{Id, Step, StepPhase};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("claude CLI not found")]
    ClaudeNotFound,
    #[error("timeout after {0} seconds")]
    Timeout(u32),
    #[error("process failed with exit code {0}")]
    ExitCode(i32),
    #[error("all retries exhausted")]
    RetriesExhausted,
}

pub type Result<T> = std::result::Result<T, RunnerError>;

/// Result of executing a step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Exit code from Claude CLI.
    pub exit_code: i32,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Path to the output log file.
    pub output_path: PathBuf,
    /// Path to the tail file (last 200 lines).
    pub tail_path: PathBuf,
    /// Output content (for completion detection).
    pub output: String,
    /// Number of retry attempts made.
    pub attempts: u32,
}

/// Runner configuration.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Model to use (e.g., "opus", "sonnet").
    pub model: String,
    /// Timeout per Claude invocation in seconds (0 = no timeout).
    pub timeout_sec: u32,
    /// Number of retries on failure (0 = no retries).
    pub retries: u32,
    /// Backoff between retries in seconds.
    pub retry_backoff_sec: u32,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            model: "opus".to_string(),
            timeout_sec: 0,
            retries: 0,
            retry_backoff_sec: 5,
        }
    }
}

impl RunnerConfig {
    /// Create from loop-core Config.
    pub fn from_config(config: &loop_core::Config) -> Self {
        Self {
            model: config.model.clone(),
            timeout_sec: config.claude_timeout_sec,
            retries: config.claude_retries,
            retry_backoff_sec: config.claude_retry_backoff_sec,
        }
    }
}

/// Runner for executing Claude CLI commands.
pub struct Runner {
    config: RunnerConfig,
}

impl Runner {
    /// Create a new runner with the given configuration.
    pub fn new(config: RunnerConfig) -> Self {
        Self { config }
    }

    /// Create a runner with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(RunnerConfig::default())
    }

    /// Get the run directory for artifacts.
    ///
    /// Follows spec Section 3.2: `<workspace_root>/logs/loop/run-<run_id>/`
    pub fn run_dir(workspace_root: &Path, run_id: &Id) -> PathBuf {
        workspace_root
            .join("logs/loop")
            .join(format!("run-{}", run_id))
    }

    /// Write the prompt file for a run.
    ///
    /// Artifact naming from spec Section 3.2: `prompt.txt`
    pub fn write_prompt(run_dir: &Path, prompt: &str) -> Result<PathBuf> {
        std::fs::create_dir_all(run_dir)?;
        let prompt_path = run_dir.join("prompt.txt");
        let mut file = std::fs::File::create(&prompt_path)?;
        file.write_all(prompt.as_bytes())?;
        Ok(prompt_path)
    }

    /// Generate iteration log path.
    ///
    /// Artifact naming from spec Section 3.2: `iter-XX.log`
    fn iter_log_path(run_dir: &Path, step: &Step) -> PathBuf {
        let iter_slug = format!("{:02}", step.attempt);
        run_dir.join(format!("iter-{}.log", iter_slug))
    }

    /// Generate iteration tail path.
    ///
    /// Artifact naming from spec Section 3.2: `iter-XX.tail.txt`
    fn iter_tail_path(run_dir: &Path, step: &Step) -> PathBuf {
        let iter_slug = format!("{:02}", step.attempt);
        run_dir.join(format!("iter-{}.tail.txt", iter_slug))
    }

    /// Execute a step with retries.
    ///
    /// Implements spec Section 4.2: `execute_step(step, prompt) -> StepResult`
    /// and Section 5.3: retry with backoff.
    pub async fn execute_step(
        &self,
        step: &Step,
        prompt: &str,
        run_dir: &Path,
        working_dir: &Path,
    ) -> Result<StepResult> {
        let max_attempts = self.config.retries + 1;
        let mut last_error: Option<RunnerError> = None;

        for attempt in 1..=max_attempts {
            info!(
                step_id = %step.id,
                phase = ?step.phase,
                attempt = attempt,
                max_attempts = max_attempts,
                "executing step"
            );

            let result = self
                .execute_single(step, prompt, run_dir, working_dir, attempt)
                .await;

            match result {
                Ok(mut step_result) => {
                    step_result.attempts = attempt;
                    return Ok(step_result);
                }
                Err(e) => {
                    warn!(
                        step_id = %step.id,
                        attempt = attempt,
                        error = %e,
                        "step execution failed"
                    );

                    last_error = Some(e);

                    if attempt < max_attempts {
                        let backoff = Duration::from_secs(self.config.retry_backoff_sec as u64);
                        info!(
                            step_id = %step.id,
                            backoff_sec = self.config.retry_backoff_sec,
                            "retrying after backoff"
                        );
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or(RunnerError::RetriesExhausted))
    }

    /// Execute a single attempt of a step.
    async fn execute_single(
        &self,
        step: &Step,
        prompt: &str,
        run_dir: &Path,
        working_dir: &Path,
        attempt: u32,
    ) -> Result<StepResult> {
        std::fs::create_dir_all(run_dir)?;

        let output_path = Self::iter_log_path(run_dir, step);
        let tail_path = Self::iter_tail_path(run_dir, step);

        let start = Utc::now();

        // Build claude command
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--dangerously-skip-permissions")
            .arg("--model")
            .arg(&self.config.model)
            .arg(prompt)
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        debug!(
            step_id = %step.id,
            model = %self.config.model,
            working_dir = %working_dir.display(),
            "spawning claude process"
        );

        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunnerError::ClaudeNotFound
            } else {
                RunnerError::Io(e)
            }
        })?;

        // Wait for process with optional timeout
        let (exit_code, stdout, stderr) = if self.config.timeout_sec > 0 {
            let timeout_duration = Duration::from_secs(self.config.timeout_sec as u64);

            match timeout(timeout_duration, child.wait_with_output()).await {
                Ok(result) => {
                    let output = result?;
                    (
                        output.status.code().unwrap_or(-1),
                        output.stdout,
                        output.stderr,
                    )
                }
                Err(_) => {
                    // Timeout - child is consumed by wait_with_output, but the process
                    // is killed when the Child is dropped, so we just return the error.
                    warn!(
                        step_id = %step.id,
                        timeout_sec = self.config.timeout_sec,
                        "process timed out"
                    );
                    return Err(RunnerError::Timeout(self.config.timeout_sec));
                }
            }
        } else {
            let output = child.wait_with_output().await?;
            (
                output.status.code().unwrap_or(-1),
                output.stdout,
                output.stderr,
            )
        };

        let end = Utc::now();
        let duration_ms = (end - start).num_milliseconds() as u64;

        // Combine stdout and stderr for the full output
        let output_content = String::from_utf8_lossy(&stdout);
        let stderr_content = String::from_utf8_lossy(&stderr);

        let full_output = if stderr.is_empty() {
            output_content.to_string()
        } else {
            format!("{}\n\n--- STDERR ---\n{}", output_content, stderr_content)
        };

        // Write output log
        {
            let mut file = std::fs::File::create(&output_path)?;
            file.write_all(full_output.as_bytes())?;
        }

        // Write tail file (last 200 lines)
        {
            let lines: Vec<&str> = full_output.lines().collect();
            let tail_start = lines.len().saturating_sub(200);
            let tail_content = lines[tail_start..].join("\n");
            let mut file = std::fs::File::create(&tail_path)?;
            file.write_all(tail_content.as_bytes())?;
        }

        info!(
            step_id = %step.id,
            exit_code = exit_code,
            duration_ms = duration_ms,
            output_bytes = full_output.len(),
            attempt = attempt,
            "step execution complete"
        );

        // Non-zero exit is an error (triggers retry)
        if exit_code != 0 {
            return Err(RunnerError::ExitCode(exit_code));
        }

        Ok(StepResult {
            exit_code,
            duration_ms,
            output_path,
            tail_path,
            output: full_output,
            attempts: attempt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::StepStatus;
    use tempfile::TempDir;

    fn create_test_step(attempt: u32) -> Step {
        Step {
            id: Id::new(),
            run_id: Id::new(),
            phase: StepPhase::Implementation,
            status: StepStatus::InProgress,
            attempt,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        }
    }

    #[test]
    fn run_dir_follows_spec_pattern() {
        let workspace = PathBuf::from("/workspace");
        let run_id = Id::from_string("test-run-123");
        let run_dir = Runner::run_dir(&workspace, &run_id);
        assert_eq!(
            run_dir,
            PathBuf::from("/workspace/logs/loop/run-test-run-123")
        );
    }

    #[test]
    fn iter_log_path_follows_spec_naming() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        let step = create_test_step(1);
        let path = Runner::iter_log_path(&run_dir, &step);
        assert_eq!(
            path,
            PathBuf::from("/workspace/logs/loop/run-123/iter-01.log")
        );
    }

    #[test]
    fn iter_tail_path_follows_spec_naming() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        let step = create_test_step(5);
        let path = Runner::iter_tail_path(&run_dir, &step);
        assert_eq!(
            path,
            PathBuf::from("/workspace/logs/loop/run-123/iter-05.tail.txt")
        );
    }

    #[test]
    fn write_prompt_creates_file() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("logs/loop/run-test");
        let prompt = "Test prompt content";

        let path = Runner::write_prompt(&run_dir, prompt).unwrap();
        assert_eq!(path, run_dir.join("prompt.txt"));
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, prompt);
    }

    #[test]
    fn runner_config_default_values() {
        let config = RunnerConfig::default();
        assert_eq!(config.model, "opus");
        assert_eq!(config.timeout_sec, 0);
        assert_eq!(config.retries, 0);
        assert_eq!(config.retry_backoff_sec, 5);
    }

    #[test]
    fn runner_config_from_loop_config() {
        let mut loop_config = loop_core::Config::default();
        loop_config.model = "sonnet".to_string();
        loop_config.claude_timeout_sec = 300;
        loop_config.claude_retries = 3;
        loop_config.claude_retry_backoff_sec = 10;

        let config = RunnerConfig::from_config(&loop_config);
        assert_eq!(config.model, "sonnet");
        assert_eq!(config.timeout_sec, 300);
        assert_eq!(config.retries, 3);
        assert_eq!(config.retry_backoff_sec, 10);
    }

    // Note: Integration tests that actually execute claude would go in a separate
    // test file or be marked #[ignore] since they require the claude CLI to be
    // installed and have external effects.
}
