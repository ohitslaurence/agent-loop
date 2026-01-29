//! HTTP client for loopd daemon.
//!
//! Communicates with loopd via its local HTTP API (Section 4.1).

use loop_core::types::{MergeStrategy, Run, RunNameSource, RunStatus, Step};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
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

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("no capacity available")]
    NoCapacity,
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() {
            ClientError::ConnectionFailed(e.to_string())
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
    pub config_path: Option<String>,
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
            return ClientError::Unauthorized("invalid or missing auth token".to_string());
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
    pub async fn tail_run(&self, run_id: &str, _follow: bool) -> Result<(), ClientError> {
        // TODO: Implement SSE streaming in a future task
        let _ = run_id;
        Err(ClientError::InvalidOperation(
            "SSE streaming not yet implemented".to_string(),
        ))
    }
}
