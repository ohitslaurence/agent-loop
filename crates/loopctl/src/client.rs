//! HTTP client for loopd daemon.
//!
//! Communicates with loopd via its local HTTP API (Section 4.1).

use loop_core::types::{MergeStrategy, Run, RunNameSource, RunStatus, Step};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("daemon not running at {addr}\n  → start with: loopd\n  → or set LOOPD_ADDR if using a different address")]
    ConnectionFailed { addr: String },

    #[error("HTTP error: {status} - {message}")]
    HttpError { status: u16, message: String },

    #[error("invalid response: {0}")]
    InvalidResponse(String),

    #[error("run not found: {0}")]
    RunNotFound(String),

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("I/O error: {0}")]
    IoError(String),

    #[error("unauthorized: check LOOPD_TOKEN env var or --token flag")]
    Unauthorized,

    #[error("no capacity: per-workspace run limit reached, wait for a run to complete")]
    NoCapacity,

    #[error(
        "daemon not ready after {timeout_ms}ms at {addr}\n  → ensure loopd is running\n  → check LOOPD_TOKEN if auth is enabled"
    )]
    DaemonNotReady { addr: String, timeout_ms: u64 },
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() {
            // Extract address from error message if possible, otherwise use placeholder
            let addr = e
                .url()
                .map(|u| u.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            ClientError::ConnectionFailed { addr }
        } else {
            ClientError::HttpError {
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                message: e.to_string(),
            }
        }
    }
}

/// Request payload for creating a run (POST /runs).
#[derive(Debug, Serialize)]
pub struct CreateRunRequest {
    pub spec_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
    pub workspace_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_source: Option<RunNameSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_branch_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_target_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_strategy: Option<MergeStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path_template: Option<String>,
}

/// Response from create run endpoint.
#[derive(Debug, Deserialize)]
pub struct CreateRunResponse {
    pub run: Run,
}

/// Response from list runs endpoint.
#[derive(Debug, Deserialize)]
pub struct ListRunsResponse {
    pub runs: Vec<Run>,
}

/// Response from get run endpoint.
#[derive(Debug, Deserialize)]
pub struct GetRunResponse {
    pub run: Run,
}

/// Response from list steps endpoint.
#[derive(Debug, Deserialize)]
pub struct ListStepsResponse {
    pub steps: Vec<Step>,
}

/// Error response from API.
#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Default total timeout for daemon readiness probe (Section 4.1).
const DEFAULT_READY_TIMEOUT_MS: u64 = 5000;

/// Initial backoff delay for readiness probe (Section 4.1).
const INITIAL_BACKOFF_MS: u64 = 200;

/// HTTP client for loopd.
pub struct Client {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: &str, token: Option<&str>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.map(String::from),
            http: reqwest::Client::new(),
        }
    }

    /// Returns the daemon address (for error messages).
    pub fn addr(&self) -> &str {
        &self.base_url
    }

    /// Check if daemon is healthy by probing /health endpoint.
    ///
    /// Returns Ok(true) if healthy, Ok(false) if unhealthy response,
    /// Err if connection failed.
    pub async fn check_health(&self) -> Result<bool, ClientError> {
        let url = format!("{}/health", self.base_url);
        let response = self.http.get(&url).headers(self.headers()).send().await?;
        Ok(response.status().is_success())
    }

    /// Wait for daemon to become ready with exponential backoff.
    ///
    /// Probes /health with retries. Per spec Section 4.1:
    /// - Retry window default: 5s total
    /// - Exponential backoff starting at 200ms
    ///
    /// Returns Ok(()) when daemon is ready, or DaemonNotReady error on timeout.
    pub async fn wait_for_ready(&self) -> Result<(), ClientError> {
        self.wait_for_ready_with_timeout(DEFAULT_READY_TIMEOUT_MS)
            .await
    }

    /// Wait for daemon to become ready with custom timeout.
    pub async fn wait_for_ready_with_timeout(&self, timeout_ms: u64) -> Result<(), ClientError> {
        let start = std::time::Instant::now();
        let mut backoff_ms = INITIAL_BACKOFF_MS;

        loop {
            match self.check_health().await {
                Ok(true) => return Ok(()),
                Ok(false) | Err(_) => {
                    let elapsed = start.elapsed().as_millis() as u64;
                    if elapsed >= timeout_ms {
                        return Err(ClientError::DaemonNotReady {
                            addr: self.base_url.clone(),
                            timeout_ms,
                        });
                    }

                    // Log retry attempt (Section 7.1: log lines when readiness probe is retrying)
                    eprintln!(
                        "waiting for daemon at {} (retrying in {}ms)",
                        self.base_url, backoff_ms
                    );

                    // Sleep for backoff duration (capped by remaining time)
                    let remaining = timeout_ms.saturating_sub(elapsed);
                    let sleep_ms = backoff_ms.min(remaining);
                    tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;

                    // Exponential backoff (double each time)
                    backoff_ms = backoff_ms.saturating_mul(2);
                }
            }
        }
    }

    /// Build headers with optional auth token.
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(token) = &self.token {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", token)) {
                headers.insert(AUTHORIZATION, value);
            }
        }
        headers
    }

    /// Handle error response from API.
    async fn handle_error(&self, response: reqwest::Response) -> ClientError {
        let status = response.status().as_u16();

        if status == 401 {
            return ClientError::Unauthorized;
        }

        if status == 404 {
            return ClientError::RunNotFound("resource not found".to_string());
        }

        if status == 503 {
            return ClientError::NoCapacity;
        }

        let message = response
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| "unknown error".to_string());

        ClientError::HttpError { status, message }
    }

    /// Create a new run.
    /// POST /runs
    pub async fn create_run(&self, req: CreateRunRequest) -> Result<Run, ClientError> {
        let url = format!("{}/runs", self.base_url);
        let response = self
            .http
            .post(&url)
            .headers(self.headers())
            .json(&req)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        let body: CreateRunResponse = response
            .json()
            .await
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        Ok(body.run)
    }

    /// List runs, optionally filtered.
    /// GET /runs?status=...&workspace_root=...
    pub async fn list_runs(
        &self,
        status: Option<RunStatus>,
        workspace_root: Option<&str>,
    ) -> Result<Vec<Run>, ClientError> {
        let mut url = format!("{}/runs", self.base_url);
        let mut params = vec![];

        if let Some(s) = status {
            params.push(format!("status={}", s.as_str()));
        }
        if let Some(ws) = workspace_root {
            params.push(format!("workspace_root={}", urlencoding::encode(ws)));
        }
        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        let response = self.http.get(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        let body: ListRunsResponse = response
            .json()
            .await
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        Ok(body.runs)
    }

    /// Get a single run.
    /// GET /runs/{id}
    pub async fn get_run(&self, run_id: &str) -> Result<Run, ClientError> {
        let url = format!("{}/runs/{}", self.base_url, run_id);
        let response = self.http.get(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        let body: GetRunResponse = response
            .json()
            .await
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        Ok(body.run)
    }

    /// List steps for a run.
    /// GET /runs/{id}/steps
    pub async fn list_steps(&self, run_id: &str) -> Result<Vec<Step>, ClientError> {
        let url = format!("{}/runs/{}/steps", self.base_url, run_id);
        let response = self.http.get(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        let body: ListStepsResponse = response
            .json()
            .await
            .map_err(|e| ClientError::InvalidResponse(e.to_string()))?;

        Ok(body.steps)
    }

    /// Pause a run.
    /// POST /runs/{id}/pause
    pub async fn pause_run(&self, run_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/runs/{}/pause", self.base_url, run_id);
        let response = self.http.post(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        Ok(())
    }

    /// Resume a run.
    /// POST /runs/{id}/resume
    pub async fn resume_run(&self, run_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/runs/{}/resume", self.base_url, run_id);
        let response = self.http.post(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        Ok(())
    }

    /// Cancel a run.
    /// POST /runs/{id}/cancel
    pub async fn cancel_run(&self, run_id: &str) -> Result<(), ClientError> {
        let url = format!("{}/runs/{}/cancel", self.base_url, run_id);
        let response = self.http.post(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        Ok(())
    }

    /// Tail run output (SSE stream).
    /// GET /runs/{id}/output
    ///
    /// Streams raw output content from run steps via Server-Sent Events.
    /// When follow=true, keeps the connection open until the run completes.
    pub async fn tail_run(&self, run_id: &str, follow: bool) -> Result<(), ClientError> {
        use futures::StreamExt;

        let url = format!("{}/runs/{}/output", self.base_url, run_id);
        let response = self.http.get(&url).headers(self.headers()).send().await?;

        if !response.status().is_success() {
            return Err(self.handle_error(response).await);
        }

        // Stream the response bytes
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ClientError::IoError(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events (separated by double newlines)
            while let Some(end) = buffer.find("\n\n") {
                let event_str = buffer[..end].to_string();
                buffer = buffer[end + 2..].to_string();

                // Parse SSE event
                if let Some(output) = parse_sse_output_event(&event_str) {
                    // Print the content directly to stdout (like tail -f)
                    print!("{}", output.content);
                }
            }

            // If not following and no more chunks pending, break after processing
            if !follow {
                // Check if stream appears done (no pending data indicator)
                // In non-follow mode, we still process the full stream for completed runs
            }
        }

        // Process any remaining buffer content
        if !buffer.is_empty() {
            if let Some(output) = parse_sse_output_event(&buffer) {
                print!("{}", output.content);
            }
        }

        Ok(())
    }
}

/// Parsed output event from SSE stream.
#[derive(Debug, Deserialize)]
struct OutputEvent {
    #[allow(dead_code)]
    step_id: String,
    #[allow(dead_code)]
    offset: u64,
    content: String,
}

/// Parse an SSE event string into OutputEvent if it's an output event.
fn parse_sse_output_event(event_str: &str) -> Option<OutputEvent> {
    let mut event_type = None;
    let mut data = None;

    for line in event_str.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            data = Some(value.trim());
        }
    }

    // Only process "output" events
    if event_type == Some("output") {
        if let Some(json_str) = data {
            return serde_json::from_str(json_str).ok();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SSE parsing tests (spec Section 7.2: tail streams live output) ---

    #[test]
    fn parse_output_event_valid() {
        let event_str = r#"event: output
data: {"step_id":"step-123","offset":0,"content":"Hello, world!\n"}"#;

        let result = parse_sse_output_event(event_str);
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output.step_id, "step-123");
        assert_eq!(output.offset, 0);
        assert_eq!(output.content, "Hello, world!\n");
    }

    #[test]
    fn parse_output_event_with_multiline_content() {
        let event_str = r#"event: output
data: {"step_id":"step-456","offset":100,"content":"Line 1\nLine 2\nLine 3\n"}"#;

        let result = parse_sse_output_event(event_str);
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output.content, "Line 1\nLine 2\nLine 3\n");
    }

    #[test]
    fn parse_output_event_ignores_non_output_events() {
        // A keepalive or comment event
        let event_str = ":keepalive";
        assert!(parse_sse_output_event(event_str).is_none());

        // A different event type
        let event_str = r#"event: status
data: {"status":"running"}"#;
        assert!(parse_sse_output_event(event_str).is_none());
    }

    #[test]
    fn parse_output_event_handles_missing_data() {
        let event_str = "event: output";
        assert!(parse_sse_output_event(event_str).is_none());
    }

    #[test]
    fn parse_output_event_handles_invalid_json() {
        let event_str = r#"event: output
data: not valid json"#;
        assert!(parse_sse_output_event(event_str).is_none());
    }

    #[test]
    fn parse_output_event_with_whitespace() {
        // Event type and data may have leading/trailing whitespace
        let event_str = r#"event:   output
data:   {"step_id":"step-789","offset":50,"content":"test"}  "#;

        let result = parse_sse_output_event(event_str);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "test");
    }

    #[test]
    fn parse_output_event_with_large_offset() {
        let event_str = r#"event: output
data: {"step_id":"step-big","offset":9999999999,"content":"at end"}"#;

        let result = parse_sse_output_event(event_str);
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output.offset, 9999999999);
    }

    // --- Client construction tests ---

    #[test]
    fn client_trims_trailing_slash() {
        let client = Client::new("http://localhost:7700/", None);
        assert_eq!(client.base_url, "http://localhost:7700");
    }

    #[test]
    fn client_preserves_url_without_trailing_slash() {
        let client = Client::new("http://localhost:7700", None);
        assert_eq!(client.base_url, "http://localhost:7700");
    }

    #[test]
    fn client_stores_auth_token() {
        let client = Client::new("http://localhost:7700", Some("my-secret-token"));
        assert_eq!(client.token, Some("my-secret-token".to_string()));
    }

    #[test]
    fn client_headers_include_content_type() {
        let client = Client::new("http://localhost:7700", None);
        let headers = client.headers();
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
    }

    #[test]
    fn client_headers_include_auth_when_token_set() {
        let client = Client::new("http://localhost:7700", Some("test-token"));
        let headers = client.headers();
        assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer test-token");
    }

    #[test]
    fn client_headers_omit_auth_when_no_token() {
        let client = Client::new("http://localhost:7700", None);
        let headers = client.headers();
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn client_addr_returns_base_url() {
        let client = Client::new("http://localhost:7700", None);
        assert_eq!(client.addr(), "http://localhost:7700");
    }

    // --- Readiness probe tests (Section 4.1) ---

    #[tokio::test]
    async fn check_health_fails_when_daemon_not_running() {
        // Connect to a port that's not listening
        let client = Client::new("http://127.0.0.1:19999", None);
        let result = client.check_health().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wait_for_ready_times_out_when_daemon_not_running() {
        let client = Client::new("http://127.0.0.1:19999", None);
        // Use a very short timeout for testing
        let result = client.wait_for_ready_with_timeout(100).await;

        match result {
            Err(ClientError::DaemonNotReady { addr, timeout_ms }) => {
                assert_eq!(addr, "http://127.0.0.1:19999");
                assert_eq!(timeout_ms, 100);
            }
            _ => panic!("expected DaemonNotReady error"),
        }
    }

    #[test]
    fn daemon_not_ready_error_message_includes_hint() {
        let err = ClientError::DaemonNotReady {
            addr: "http://127.0.0.1:7700".to_string(),
            timeout_ms: 5000,
        };
        let msg = err.to_string();
        assert!(msg.contains("127.0.0.1:7700"));
        assert!(msg.contains("5000ms"));
        assert!(msg.contains("LOOPD_TOKEN"));
    }

    // --- Improved error message tests (Section 6.1) ---

    #[test]
    fn connection_failed_error_suggests_start_command() {
        let err = ClientError::ConnectionFailed {
            addr: "http://127.0.0.1:7700".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("loopd"), "should suggest starting loopd");
        assert!(
            msg.contains("LOOPD_ADDR"),
            "should mention LOOPD_ADDR env var"
        );
    }

    #[test]
    fn unauthorized_error_suggests_token_options() {
        let err = ClientError::Unauthorized;
        let msg = err.to_string();
        assert!(
            msg.contains("LOOPD_TOKEN"),
            "should mention LOOPD_TOKEN env var"
        );
        assert!(msg.contains("--token"), "should mention --token flag");
    }

    #[test]
    fn no_capacity_error_explains_cause() {
        let err = ClientError::NoCapacity;
        let msg = err.to_string();
        assert!(
            msg.contains("per-workspace") || msg.contains("limit"),
            "should explain capacity limit"
        );
    }
}
