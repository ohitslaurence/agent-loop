//! Runner module for executing Claude CLI with retries and timeouts.
//!
//! Implements step execution via the Claude CLI (spec Section 4.2, 5.3, 7.1).
//! Key responsibilities:
//! - Execute Claude CLI with configurable timeout
//! - Retry on failure with exponential backoff
//! - Write artifacts (iter-XX.log, iter-XX.tail.txt)
//! - Track step timing and exit codes

use chrono::Utc;
use loop_core::{Id, Step};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Interval between heartbeat log messages during long-running Claude executions.
///
/// Helps operators monitor progress and identify stuck processes.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Timeout for capturing stdout/stderr after process exits or is killed.
///
/// Normally I/O completes immediately after process death, but if pipes
/// are backed up or there's a bug, we don't want to hang forever.
const IO_CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum bytes to capture from stdout/stderr.
///
/// Prevents OOM if Claude produces excessive output. 50MB is generous
/// for normal operation but prevents runaway memory usage.
const MAX_OUTPUT_BYTES: usize = 50 * 1024 * 1024;

/// Number of lines to include in the tail file.
///
/// The tail file provides a quick view of recent output without
/// loading the entire log. 200 lines balances context with file size.
const TAIL_LINES: usize = 200;

/// Read from an async reader with a maximum byte limit.
///
/// Returns the buffer truncated at `max_bytes`. Logs a warning if truncated.
async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];

    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(buf.len());
        if remaining == 0 {
            // Already at limit, drain remaining input
            tracing::warn!(max_bytes, "output exceeded limit, truncating");
            // Keep reading to drain the pipe but discard
            while reader.read(&mut chunk).await? > 0 {}
            break;
        }

        let to_take = n.min(remaining);
        buf.extend_from_slice(&chunk[..to_take]);
    }

    Ok(buf)
}

/// Read Claude `--output-format stream-json` output, extract text, stream to disk.
///
/// Claude's stream-json format produces newline-delimited JSON events following the
/// Anthropic API streaming protocol. We extract text from `content_block_delta` events
/// (where `delta.type == "text_delta"`) and write it to the log file as each chunk arrives.
/// Returns the accumulated plain-text content.
async fn stream_claude_json<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    max_bytes: usize,
    path: PathBuf,
) -> std::io::Result<Vec<u8>> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .await?;

    let mut buf_reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    let mut text_buf = Vec::with_capacity(8192);
    let mut truncated = false;

    loop {
        line.clear();
        let n = buf_reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse JSON event; extract text from content_block_delta with text_delta.
        let text = match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(event) => {
                if event.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                    event
                        .get("delta")
                        .and_then(|d| {
                            if d.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                                d.get("text").and_then(|t| t.as_str())
                            } else {
                                None
                            }
                        })
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            Err(err) => {
                tracing::debug!(line = trimmed, error = %err, "ignoring unparseable stream-json line");
                None
            }
        };

        if let Some(text) = text {
            let bytes = text.as_bytes();

            // Write to disk immediately so partial output survives timeouts.
            file.write_all(bytes).await?;

            // Accumulate in memory up to the limit.
            if !truncated {
                let remaining = max_bytes.saturating_sub(text_buf.len());
                if remaining == 0 {
                    tracing::warn!(max_bytes, "stream-json text exceeded limit, truncating in-memory buffer");
                    truncated = true;
                } else {
                    let to_take = bytes.len().min(remaining);
                    text_buf.extend_from_slice(&bytes[..to_take]);
                    if to_take < bytes.len() {
                        truncated = true;
                    }
                }
            }
        }
    }

    file.flush().await?;
    Ok(text_buf)
}

/// How the process wait loop terminated.
enum ProcessOutcome {
    Completed(std::process::ExitStatus),
    TimedOut,
    Cancelled,
}

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
    #[error("cancelled")]
    Cancelled,
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
            timeout_sec: 600,
            retries: 0,
            retry_backoff_sec: 5,
        }
    }
}

impl RunnerConfig {
    /// Create from loop-core Config (uses `model` field).
    pub fn from_config(config: &loop_core::Config) -> Self {
        Self {
            model: config.model.clone(),
            timeout_sec: config.claude_timeout_sec,
            retries: config.claude_retries,
            retry_backoff_sec: config.claude_retry_backoff_sec,
        }
    }

    /// Create from loop-core Config for review steps.
    ///
    /// Uses `review_model` if set, otherwise falls back to `model`.
    pub fn from_config_for_review(config: &loop_core::Config) -> Self {
        Self {
            model: config
                .review_model
                .clone()
                .unwrap_or_else(|| config.model.clone()),
            timeout_sec: config.claude_timeout_sec,
            retries: config.claude_retries,
            retry_backoff_sec: config.claude_retry_backoff_sec,
        }
    }
}

/// Runner for executing Claude CLI commands.
#[derive(Debug)]
pub struct Runner {
    config: RunnerConfig,
}

/// Truncate a string for logging, adding "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
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
            .join(format!("run-{run_id}"))
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
    /// Artifact naming: `iter-XX-phase.log` (e.g., iter-01-impl.log)
    fn iter_log_path(run_dir: &Path, step: &Step) -> PathBuf {
        run_dir.join(format!(
            "iter-{:02}-{}.log",
            step.attempt,
            step.phase.slug()
        ))
    }

    /// Generate iteration tail path.
    ///
    /// Artifact naming: `iter-XX-phase.tail.txt` (e.g., iter-01-impl.tail.txt)
    fn iter_tail_path(run_dir: &Path, step: &Step) -> PathBuf {
        run_dir.join(format!(
            "iter-{:02}-{}.tail.txt",
            step.attempt,
            step.phase.slug()
        ))
    }

    /// Execute a step with retries.
    ///
    /// Implements spec Section 4.2: `execute_step(step, prompt) -> StepResult`
    /// and Section 5.3: retry with backoff.
    ///
    /// If `cancel_token` is cancelled, the step will be aborted and return `Cancelled`.
    pub async fn execute_step(
        &self,
        step: &Step,
        prompt: &str,
        run_dir: &Path,
        working_dir: &Path,
        cancel_token: CancellationToken,
    ) -> Result<StepResult> {
        let max_attempts = self.config.retries + 1;
        let mut last_error: Option<RunnerError> = None;

        for retry in 1..=max_attempts {
            info!(
                step_id = %step.id,
                phase = ?step.phase,
                step_attempt = step.attempt,
                retry = retry,
                max_retries = max_attempts,
                working_dir = %working_dir.display(),
                "executing step"
            );

            let result = self
                .execute_single(
                    step,
                    prompt,
                    run_dir,
                    working_dir,
                    retry,
                    cancel_token.clone(),
                )
                .await;

            // Don't retry if cancelled
            if matches!(result, Err(RunnerError::Cancelled)) {
                return result;
            }

            match result {
                Ok(mut step_result) => {
                    step_result.attempts = retry;
                    return Ok(step_result);
                }
                Err(e) => {
                    warn!(
                        step_id = %step.id,
                        retry = retry,
                        error = %e,
                        "step execution failed"
                    );

                    last_error = Some(e);

                    if retry < max_attempts {
                        let backoff = Duration::from_secs(u64::from(self.config.retry_backoff_sec));
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

    /// Execute a single retry of a step.
    ///
    /// Stdout is streamed to the log file as it arrives so partial output
    /// survives timeouts and cancellations.
    async fn execute_single(
        &self,
        step: &Step,
        prompt: &str,
        run_dir: &Path,
        working_dir: &Path,
        retry: u32,
        cancel_token: CancellationToken,
    ) -> Result<StepResult> {
        std::fs::create_dir_all(run_dir)?;

        let output_path = Self::iter_log_path(run_dir, step);
        let tail_path = Self::iter_tail_path(run_dir, step);

        let start = Utc::now();

        // Build claude command
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg("--verbose")
            .arg("--dangerously-skip-permissions")
            .arg("--output-format")
            .arg("stream-json")
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

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunnerError::ClaudeNotFound
            } else {
                RunnerError::Io(e)
            }
        })?;

        // Parse stream-json stdout, extract text, stream to log file as it arrives.
        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| tokio::spawn(stream_claude_json(stdout, MAX_OUTPUT_BYTES, output_path.clone())));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(read_bounded(stderr, MAX_OUTPUT_BYTES)));

        // Wait for process with periodic progress logging.
        let started = Instant::now();
        let timeout_duration = Duration::from_secs(u64::from(self.config.timeout_sec));

        let outcome = loop {
            let elapsed = started.elapsed();

            // Check timeout (only if timeout is configured)
            if self.config.timeout_sec > 0 && elapsed >= timeout_duration {
                warn!(
                    step_id = %step.id,
                    timeout_sec = self.config.timeout_sec,
                    "process timed out; killing"
                );
                if let Err(err) = child.kill().await {
                    warn!(
                        step_id = %step.id,
                        error = %err,
                        "failed to kill timed-out process"
                    );
                }
                let _ = child.wait().await;
                break ProcessOutcome::TimedOut;
            }

            // Calculate sleep duration: min of heartbeat interval and remaining timeout
            let remaining_timeout = if self.config.timeout_sec > 0 {
                timeout_duration.saturating_sub(elapsed)
            } else {
                Duration::MAX
            };
            let sleep_duration = HEARTBEAT_INTERVAL.min(remaining_timeout);

            tokio::select! {
                result = child.wait() => {
                    match result {
                        Ok(status) => break ProcessOutcome::Completed(status),
                        Err(e) => return Err(RunnerError::Io(e)),
                    }
                }
                () = cancel_token.cancelled() => {
                    info!(
                        step_id = %step.id,
                        "cancellation requested; killing process"
                    );
                    if let Err(err) = child.kill().await {
                        warn!(
                            step_id = %step.id,
                            error = %err,
                            "failed to kill cancelled process"
                        );
                    }
                    let _ = child.wait().await;
                    break ProcessOutcome::Cancelled;
                }
                () = tokio::time::sleep(sleep_duration) => {
                    let elapsed_secs = started.elapsed().as_secs();
                    info!(
                        step_id = %step.id,
                        phase = ?step.phase,
                        elapsed_sec = elapsed_secs,
                        timeout_sec = self.config.timeout_sec,
                        working_dir = %working_dir.display(),
                        "claude still running"
                    );
                }
            }
        };

        // Always capture remaining output (pipe closes after kill, tasks finish quickly).
        let stdout = match stdout_task {
            Some(task) => match timeout(IO_CAPTURE_TIMEOUT, task).await {
                Ok(Ok(Ok(buf))) => buf,
                Ok(Ok(Err(err))) => {
                    warn!(step_id = %step.id, error = %err, "stdout capture failed");
                    Vec::new()
                }
                Ok(Err(err)) => {
                    warn!(step_id = %step.id, error = %err, "stdout task panicked");
                    Vec::new()
                }
                Err(_) => {
                    warn!(step_id = %step.id, "stdout capture timed out");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let stderr = match stderr_task {
            Some(task) => match timeout(IO_CAPTURE_TIMEOUT, task).await {
                Ok(Ok(Ok(buf))) => buf,
                Ok(Ok(Err(err))) => {
                    warn!(step_id = %step.id, error = %err, "stderr capture failed");
                    Vec::new()
                }
                Ok(Err(err)) => {
                    warn!(step_id = %step.id, error = %err, "stderr task panicked");
                    Vec::new()
                }
                Err(_) => {
                    warn!(step_id = %step.id, "stderr capture timed out");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        let end = Utc::now();
        let duration_ms = (end - start).num_milliseconds() as u64;

        // Build full output string (stdout already on disk via tee).
        let output_content = String::from_utf8_lossy(&stdout);
        let stderr_content = String::from_utf8_lossy(&stderr);

        let full_output = if stderr.is_empty() {
            output_content.to_string()
        } else {
            format!("{output_content}\n\n--- STDERR ---\n{stderr_content}")
        };

        // Append stderr to the log file if present (stdout was already streamed).
        if !stderr.is_empty() {
            if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&output_path) {
                let _ = file.write_all(b"\n\n--- STDERR ---\n");
                let _ = file.write_all(&stderr);
            }
        }

        // Write tail file (last 200 lines).
        {
            let lines: Vec<&str> = full_output.lines().collect();
            let tail_start = lines.len().saturating_sub(TAIL_LINES);
            let tail_content = lines[tail_start..].join("\n");
            if let Ok(mut file) = std::fs::File::create(&tail_path) {
                let _ = file.write_all(tail_content.as_bytes());
            }
        }

        // Log completion with output preview.
        let output_preview = {
            let lines: Vec<&str> = full_output.lines().collect();
            let last_lines: Vec<&str> = lines.iter().rev().take(3).copied().collect();
            last_lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
        };

        match outcome {
            ProcessOutcome::TimedOut => {
                info!(
                    step_id = %step.id,
                    phase = ?step.phase,
                    duration_ms = duration_ms,
                    output_bytes = full_output.len(),
                    "step timed out (partial output saved)"
                );
                Err(RunnerError::Timeout(self.config.timeout_sec))
            }
            ProcessOutcome::Cancelled => {
                info!(
                    step_id = %step.id,
                    phase = ?step.phase,
                    duration_ms = duration_ms,
                    output_bytes = full_output.len(),
                    "step cancelled (partial output saved)"
                );
                Err(RunnerError::Cancelled)
            }
            ProcessOutcome::Completed(exit_status) => {
                let exit_code = exit_status.code().unwrap_or(-1);

                info!(
                    step_id = %step.id,
                    phase = ?step.phase,
                    exit_code = exit_code,
                    duration_ms = duration_ms,
                    output_bytes = full_output.len(),
                    output_lines = full_output.lines().count(),
                    output_preview = %truncate_str(&output_preview, 120),
                    "step execution complete"
                );

                if exit_code != 0 {
                    return Err(RunnerError::ExitCode(exit_code));
                }

                Ok(StepResult {
                    exit_code,
                    duration_ms,
                    output_path,
                    tail_path,
                    output: full_output,
                    attempts: retry,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::{Id, StepPhase, StepStatus};
    use tempfile::TempDir;
    use tokio::time::timeout;

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
    fn iter_log_path_includes_phase() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        let step = create_test_step(1);
        let path = Runner::iter_log_path(&run_dir, &step);
        assert_eq!(
            path,
            PathBuf::from("/workspace/logs/loop/run-123/iter-01-impl.log")
        );
    }

    #[test]
    fn iter_tail_path_includes_phase() {
        let run_dir = PathBuf::from("/workspace/logs/loop/run-123");
        let step = create_test_step(5);
        let path = Runner::iter_tail_path(&run_dir, &step);
        assert_eq!(
            path,
            PathBuf::from("/workspace/logs/loop/run-123/iter-05-impl.tail.txt")
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
        assert_eq!(config.timeout_sec, 600);
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

    #[test]
    fn runner_config_for_review_uses_review_model() {
        let mut loop_config = loop_core::Config::default();
        loop_config.model = "sonnet".to_string();
        loop_config.review_model = Some("opus".to_string());

        let config = RunnerConfig::from_config_for_review(&loop_config);
        assert_eq!(config.model, "opus");
    }

    #[test]
    fn runner_config_for_review_falls_back_to_model() {
        let mut loop_config = loop_core::Config::default();
        loop_config.model = "sonnet".to_string();
        loop_config.review_model = None;

        let config = RunnerConfig::from_config_for_review(&loop_config);
        assert_eq!(config.model, "sonnet");
    }

    // Note: Integration tests that actually execute claude would go in a separate
    // test file or be marked #[ignore] since they require the claude CLI to be
    // installed and have external effects.

    // -------------------------------------------------------------------------
    // Tests for retry/timeout/exit handling (spec Section 5.3, Section 6.2)
    // -------------------------------------------------------------------------

    /// Test runner that uses a custom command instead of 'claude'.
    /// This allows testing retry and timeout behavior without the real CLI.
    struct TestRunner {
        config: RunnerConfig,
        command: String,
        args: Vec<String>,
    }

    impl TestRunner {
        fn new(config: RunnerConfig, command: &str, args: Vec<String>) -> Self {
            Self {
                config,
                command: command.to_string(),
                args,
            }
        }

        /// Execute step with custom command (mirrors Runner::execute_step logic).
        async fn execute_step(
            &self,
            step: &Step,
            run_dir: &Path,
            working_dir: &Path,
        ) -> Result<StepResult> {
            let max_attempts = self.config.retries + 1;
            let mut last_error: Option<RunnerError> = None;

            for attempt in 1..=max_attempts {
                let result = self
                    .execute_single(step, run_dir, working_dir, attempt)
                    .await;

                match result {
                    Ok(mut step_result) => {
                        step_result.attempts = attempt;
                        return Ok(step_result);
                    }
                    Err(e) => {
                        last_error = Some(e);
                        if attempt < max_attempts {
                            let backoff = Duration::from_millis(
                                (self.config.retry_backoff_sec * 10) as u64, // 10ms per "second" for fast tests
                            );
                            tokio::time::sleep(backoff).await;
                        }
                    }
                }
            }

            Err(last_error.unwrap_or(RunnerError::RetriesExhausted))
        }

        async fn execute_single(
            &self,
            step: &Step,
            run_dir: &Path,
            working_dir: &Path,
            attempt: u32,
        ) -> Result<StepResult> {
            std::fs::create_dir_all(run_dir)?;

            let output_path = Runner::iter_log_path(run_dir, step);
            let tail_path = Runner::iter_tail_path(run_dir, step);

            let start = Utc::now();

            let mut cmd = Command::new(&self.command);
            for arg in &self.args {
                cmd.arg(arg);
            }
            cmd.current_dir(working_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let child = cmd.spawn().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RunnerError::ClaudeNotFound
                } else {
                    RunnerError::Io(e)
                }
            })?;

            let (exit_code, stdout, stderr) = if self.config.timeout_sec > 0 {
                // Use milliseconds for timeout in tests (timeout_sec treated as ms)
                let timeout_duration = Duration::from_millis(self.config.timeout_sec as u64);

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

            let output_content = String::from_utf8_lossy(&stdout);
            let stderr_content = String::from_utf8_lossy(&stderr);

            let full_output = if stderr.is_empty() {
                output_content.to_string()
            } else {
                format!("{}\n\n--- STDERR ---\n{}", output_content, stderr_content)
            };

            {
                let mut file = std::fs::File::create(&output_path)?;
                file.write_all(full_output.as_bytes())?;
            }

            {
                let lines: Vec<&str> = full_output.lines().collect();
                let tail_start = lines.len().saturating_sub(TAIL_LINES);
                let tail_content = lines[tail_start..].join("\n");
                let mut file = std::fs::File::create(&tail_path)?;
                file.write_all(tail_content.as_bytes())?;
            }

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

    #[tokio::test]
    async fn execute_step_succeeds_on_zero_exit() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // 'true' command exits with 0
        let config = RunnerConfig {
            retries: 0,
            ..Default::default()
        };
        let runner = TestRunner::new(config, "true", vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.attempts, 1);
    }

    #[tokio::test]
    async fn execute_step_fails_on_nonzero_exit_no_retries() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // 'false' command exits with 1
        let config = RunnerConfig {
            retries: 0,
            ..Default::default()
        };
        let runner = TestRunner::new(config, "false", vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RunnerError::ExitCode(code) => assert_eq!(code, 1),
            e => panic!("expected ExitCode error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn execute_step_retries_on_failure() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // Create a script that fails twice then succeeds (using unique file per test)
        let script_path = dir.path().join("retry_script.sh");
        let counter_path = dir.path().join("counter_retry");
        // Initialize counter to 0
        std::fs::write(&counter_path, "0").unwrap();
        std::fs::write(
            &script_path,
            format!(
                r#"#!/bin/sh
counter=$(cat "{counter}")
counter=$((counter + 1))
echo $counter > "{counter}"
# Fail on attempts 1 and 2, succeed on attempt 3
if [ $counter -le 2 ]; then
    exit 1
fi
echo "success"
exit 0
"#,
                counter = counter_path.display(),
            ),
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config = RunnerConfig {
            retries: 3,           // Allow up to 4 attempts (1 + 3 retries)
            retry_backoff_sec: 1, // 10ms in test (multiplied by 10 in TestRunner)
            ..Default::default()
        };
        let runner = TestRunner::new(config, script_path.to_str().unwrap(), vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok(), "expected success, got {:?}", result);
        let result = result.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.attempts, 3,
            "expected 3 attempts (failed 2, succeeded on 3rd)"
        );
    }

    #[tokio::test]
    async fn execute_step_exhausts_retries() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // 'false' always fails
        let config = RunnerConfig {
            retries: 2, // 3 total attempts
            retry_backoff_sec: 1,
            ..Default::default()
        };
        let runner = TestRunner::new(config, "false", vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_err());
        // Last error should be ExitCode
        match result.unwrap_err() {
            RunnerError::ExitCode(code) => assert_eq!(code, 1),
            e => panic!(
                "expected ExitCode error after retries exhausted, got {:?}",
                e
            ),
        }
    }

    #[tokio::test]
    async fn execute_step_times_out() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // sleep command that exceeds timeout
        let config = RunnerConfig {
            timeout_sec: 50, // 50ms timeout in test mode
            retries: 0,
            ..Default::default()
        };
        let runner = TestRunner::new(config, "sleep", vec!["1".to_string()]); // sleep 1 second

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RunnerError::Timeout(t) => assert_eq!(t, 50),
            e => panic!("expected Timeout error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn execute_step_timeout_triggers_retry() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // Create script that sleeps first time, then succeeds
        let script_path = dir.path().join("timeout_retry.sh");
        let counter_path = dir.path().join("counter2");
        std::fs::write(&counter_path, "0").unwrap();
        std::fs::write(
            &script_path,
            format!(
                r#"#!/bin/sh
counter=$(cat "{}")
counter=$((counter + 1))
echo $counter > "{}"
if [ $counter -eq 1 ]; then
    sleep 2
fi
echo "success"
exit 0
"#,
                counter_path.display(),
                counter_path.display()
            ),
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config = RunnerConfig {
            timeout_sec: 50, // 50ms timeout
            retries: 1,
            retry_backoff_sec: 1,
            ..Default::default()
        };
        let runner = TestRunner::new(config, script_path.to_str().unwrap(), vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.attempts, 2); // First timed out, second succeeded
    }

    #[tokio::test]
    async fn execute_step_command_not_found() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        let config = RunnerConfig::default();
        let runner = TestRunner::new(config, "nonexistent_command_xyz", vec![]);

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            RunnerError::ClaudeNotFound => {} // Expected
            e => panic!("expected ClaudeNotFound error, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn execute_step_writes_output_artifacts() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(3);

        // Echo some output
        let config = RunnerConfig::default();
        let runner = TestRunner::new(
            config,
            "sh",
            vec!["-c".to_string(), "echo 'test output'".to_string()],
        );

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok());
        let result = result.unwrap();

        // Check artifacts were written with correct naming (iter-03-impl.log, iter-03-impl.tail.txt)
        assert!(result.output_path.exists());
        assert!(result.tail_path.exists());
        assert_eq!(result.output_path.file_name().unwrap(), "iter-03-impl.log");
        assert_eq!(
            result.tail_path.file_name().unwrap(),
            "iter-03-impl.tail.txt"
        );

        let output_content = std::fs::read_to_string(&result.output_path).unwrap();
        assert!(output_content.contains("test output"));
    }

    #[tokio::test]
    async fn execute_step_captures_stderr() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // Write to both stdout and stderr
        let config = RunnerConfig::default();
        let runner = TestRunner::new(
            config,
            "sh",
            vec![
                "-c".to_string(),
                "echo 'stdout'; echo 'stderr' >&2".to_string(),
            ],
        );

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok());
        let result = result.unwrap();

        assert!(result.output.contains("stdout"));
        assert!(result.output.contains("--- STDERR ---"));
        assert!(result.output.contains("stderr"));
    }

    #[tokio::test]
    async fn execute_step_tracks_duration() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // Sleep briefly to have measurable duration
        let config = RunnerConfig::default();
        let runner = TestRunner::new(
            config,
            "sh",
            vec!["-c".to_string(), "sleep 0.05".to_string()],
        );

        let result = runner.execute_step(&step, &run_dir, dir.path()).await;
        assert!(result.is_ok());
        let result = result.unwrap();

        // Should be at least 50ms
        assert!(result.duration_ms >= 50);
    }

    #[tokio::test]
    async fn execute_step_cancelled_kills_process() {
        let dir = TempDir::new().unwrap();
        let run_dir = dir.path().join("run-test");
        let step = create_test_step(1);

        // Create a runner that will run a long sleep
        let config = RunnerConfig {
            timeout_sec: 0, // No timeout
            retries: 0,
            ..Default::default()
        };
        let runner = Runner::new(config);

        // Create a cancel token and cancel it after a short delay
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel_token_clone.cancel();
        });

        // Run a long sleep - should be cancelled
        let result = runner
            .execute_step(&step, "sleep 10", &run_dir, dir.path(), cancel_token)
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            RunnerError::Cancelled => {} // Expected
            e => panic!("expected Cancelled, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn stream_claude_json_extracts_text_deltas() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("output.log");

        // Simulate Claude stream-json output with multiple event types.
        let stream_data = concat!(
            r#"{"type":"message_start","message":{"id":"msg_01","role":"assistant","content":[],"model":"claude-sonnet-4-20250514"}}"#, "\n",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#, "\n",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#, "\n",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}"#, "\n",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}"#, "\n",
            r#"{"type":"content_block_stop","index":0}"#, "\n",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#, "\n",
            r#"{"type":"message_stop"}"#, "\n",
        );

        let reader = tokio::io::BufReader::new(stream_data.as_bytes());
        let result = stream_claude_json(reader, MAX_OUTPUT_BYTES, log_path.clone()).await.unwrap();

        let text = String::from_utf8(result).unwrap();
        assert_eq!(text, "Hello world!");

        // Log file should contain the same extracted text.
        let on_disk = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(on_disk, "Hello world!");
    }

    #[tokio::test]
    async fn stream_claude_json_handles_non_text_deltas() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("output.log");

        // Include a tool_use delta that should be ignored.
        let stream_data = concat!(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#, "\n",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#, "\n",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}"#, "\n",
        );

        let reader = tokio::io::BufReader::new(stream_data.as_bytes());
        let result = stream_claude_json(reader, MAX_OUTPUT_BYTES, log_path.clone()).await.unwrap();

        assert_eq!(String::from_utf8(result).unwrap(), "ok!");
    }

    #[tokio::test]
    async fn stream_claude_json_truncates_at_limit() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("output.log");

        let stream_data = concat!(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"abcde"}}"#, "\n",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"fghij"}}"#, "\n",
        );

        let reader = tokio::io::BufReader::new(stream_data.as_bytes());
        // Limit in-memory to 7 bytes.
        let result = stream_claude_json(reader, 7, log_path.clone()).await.unwrap();

        // In-memory buffer truncated at 7 bytes.
        assert_eq!(String::from_utf8(result).unwrap(), "abcdefg");

        // Disk should have all 10 bytes.
        let on_disk = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(on_disk, "abcdefghij");
    }
}
