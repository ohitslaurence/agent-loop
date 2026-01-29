//! HTTP client for loopd daemon.
//!
//! Communicates with loopd via its local HTTP API (Section 4.1).
//! Currently stubbed until HTTP server is implemented.

use loop_core::types::{MergeStrategy, Run, RunNameSource, RunStatus, Step};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("daemon not running or unreachable at {0}")]
    ConnectionFailed(String),

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

    #[error("server not implemented - HTTP API not yet available")]
    NotImplemented,
}

/// Request payload for creating a run (POST /runs).
#[derive(Debug, Serialize)]
pub struct CreateRunRequest {
    pub spec_path: String,
    pub plan_path: Option<String>,
    pub workspace_root: String,
    pub config_path: Option<String>,
    pub name: Option<String>,
    pub name_source: Option<RunNameSource>,
    pub base_branch: Option<String>,
    pub run_branch_prefix: Option<String>,
    pub merge_target_branch: Option<String>,
    pub merge_strategy: Option<MergeStrategy>,
    pub worktree_path_template: Option<String>,
}

/// Response from list runs endpoint.
#[derive(Debug, Deserialize)]
pub struct ListRunsResponse {
    pub runs: Vec<Run>,
}

/// Response from list steps endpoint.
#[derive(Debug, Deserialize)]
pub struct ListStepsResponse {
    pub steps: Vec<Step>,
}

/// HTTP client for loopd.
pub struct Client {
    base_url: String,
    #[allow(dead_code)]
    token: Option<String>,
}

impl Client {
    pub fn new(base_url: &str, token: Option<&str>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.map(String::from),
        }
    }

    /// Create a new run.
    /// POST /runs
    pub async fn create_run(&self, _req: CreateRunRequest) -> Result<Run, ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{} - HTTP server not yet implemented",
            self.base_url
        )))
    }

    /// List runs, optionally filtered.
    /// GET /runs?status=...&workspace_root=...
    pub async fn list_runs(
        &self,
        _status: Option<RunStatus>,
        _workspace_root: Option<&str>,
    ) -> Result<Vec<Run>, ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{} - HTTP server not yet implemented",
            self.base_url
        )))
    }

    /// Get a single run.
    /// GET /runs/{id}
    pub async fn get_run(&self, run_id: &str) -> Result<Run, ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{} - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }

    /// List steps for a run.
    /// GET /runs/{id}/steps
    pub async fn list_steps(&self, run_id: &str) -> Result<Vec<Step>, ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{}/steps - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }

    /// Pause a run.
    /// POST /runs/{id}/pause
    pub async fn pause_run(&self, run_id: &str) -> Result<(), ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{}/pause - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }

    /// Resume a run.
    /// POST /runs/{id}/resume
    pub async fn resume_run(&self, run_id: &str) -> Result<(), ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{}/resume - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }

    /// Cancel a run.
    /// POST /runs/{id}/cancel
    pub async fn cancel_run(&self, run_id: &str) -> Result<(), ClientError> {
        // TODO: Implement when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{}/cancel - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }

    /// Tail run output (SSE stream).
    /// GET /runs/{id}/output
    pub async fn tail_run(&self, run_id: &str, _follow: bool) -> Result<(), ClientError> {
        // TODO: Implement SSE streaming when HTTP server is available
        Err(ClientError::ConnectionFailed(format!(
            "{}/runs/{}/output - HTTP server not yet implemented",
            self.base_url, run_id
        )))
    }
}
