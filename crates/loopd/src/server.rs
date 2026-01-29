//! HTTP control plane server for loopd.
//!
//! Implements the local-only REST API from spec Section 4.1.
//! See also Section 8.1 for auth requirements.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use loop_core::{Id, MergeStrategy, Run, RunNameSource, RunStatus};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::scheduler::Scheduler;
use crate::storage::Storage;

/// Shared state for HTTP handlers.
pub struct AppState {
    pub storage: Arc<Storage>,
    pub scheduler: Arc<Scheduler>,
    pub auth_token: Option<String>,
}

/// Create the HTTP router with all endpoints.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        // REST endpoints (Section 4.1)
        .route("/runs", post(create_run).get(list_runs))
        .route("/runs/{id}", get(get_run))
        .route("/runs/{id}/pause", post(pause_run))
        .route("/runs/{id}/resume", post(resume_run))
        .route("/runs/{id}/cancel", post(cancel_run))
        .route("/runs/{id}/steps", get(list_steps))
        // Health check
        .route("/health", get(health_check))
        .with_state(state)
}

/// Start the HTTP server.
pub async fn start_server(
    storage: Arc<Storage>,
    scheduler: Arc<Scheduler>,
    port: u16,
    auth_token: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(AppState {
        storage,
        scheduler,
        auth_token,
    });

    let router = create_router(state);

    // Bind to localhost only (Section 8.1: Local-only HTTP server bound to 127.0.0.1)
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    info!("HTTP server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

/// Validate auth token if configured.
fn check_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Some(expected) = &state.auth_token {
        let provided = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("Bearer ").unwrap_or(s));

        match provided {
            Some(token) if token == expected => Ok(()),
            Some(_) => Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "invalid auth token".to_string(),
                }),
            )),
            None => Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "missing auth token".to_string(),
                }),
            )),
        }
    } else {
        Ok(())
    }
}

// --- Request/Response types ---

/// Error response body.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Request payload for POST /runs (Section 4.1).
#[derive(Debug, Deserialize)]
pub struct CreateRunRequest {
    pub spec_path: String,
    #[serde(default)]
    pub plan_path: Option<String>,
    pub workspace_root: String,
    #[serde(default)]
    pub config_override: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_source: Option<RunNameSource>,
    #[serde(default)]
    pub merge_target_branch: Option<String>,
    #[serde(default)]
    pub merge_strategy: Option<MergeStrategy>,
}

/// Response for POST /runs.
#[derive(Debug, Serialize)]
pub struct CreateRunResponse {
    pub run: Run,
}

/// Query params for GET /runs.
#[derive(Debug, Deserialize, Default)]
pub struct ListRunsQuery {
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Response for GET /runs.
#[derive(Debug, Serialize)]
pub struct ListRunsResponse {
    pub runs: Vec<Run>,
}

/// Response for GET /runs/{id}.
#[derive(Debug, Serialize)]
pub struct GetRunResponse {
    pub run: Run,
}

/// Response for GET /runs/{id}/steps.
#[derive(Debug, Serialize)]
pub struct ListStepsResponse {
    pub steps: Vec<loop_core::Step>,
}

// --- Handlers ---

/// Health check endpoint.
async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

/// POST /runs - Create a new run.
async fn create_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateRunRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    // Determine run name
    let name_source = req.name_source.unwrap_or(RunNameSource::SpecSlug);
    let name = req.name.unwrap_or_else(|| {
        // Generate name from spec path
        let spec_name = PathBuf::from(&req.spec_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();
        // Sanitize for use as name
        spec_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect()
    });

    let now = Utc::now();
    let run = Run {
        id: Id::new(),
        name,
        name_source,
        status: RunStatus::Pending,
        workspace_root: req.workspace_root,
        spec_path: req.spec_path,
        plan_path: req.plan_path,
        worktree: None, // Worktree setup happens when run starts
        config_json: req.config_override,
        created_at: now,
        updated_at: now,
    };

    state.storage.insert_run(&run).await.map_err(|e| {
        error!("failed to create run: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to create run: {}", e),
            }),
        )
    })?;

    info!("created run: {} ({})", run.name, run.id);
    Ok((StatusCode::CREATED, Json(CreateRunResponse { run })))
}

/// GET /runs - List runs.
async fn list_runs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let mut runs = state
        .storage
        .list_runs(query.workspace_root.as_deref())
        .await
        .map_err(|e| {
            error!("failed to list runs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to list runs: {}", e),
                }),
            )
        })?;

    // Filter by status if provided
    if let Some(status_filter) = &query.status {
        runs.retain(|r| r.status.as_str().eq_ignore_ascii_case(status_filter));
    }

    Ok(Json(ListRunsResponse { runs }))
}

/// GET /runs/{id} - Get a single run.
async fn get_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {}", e),
            }),
        )
    })?;

    Ok(Json(GetRunResponse { run }))
}

/// GET /runs/{id}/steps - List steps for a run.
async fn list_steps(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    // Verify run exists first
    state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {}", e),
            }),
        )
    })?;

    let steps = state.storage.list_steps(&run_id).await.map_err(|e| {
        error!("failed to list steps: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("failed to list steps: {}", e),
            }),
        )
    })?;

    Ok(Json(ListStepsResponse { steps }))
}

/// POST /runs/{id}/pause - Pause a running run.
async fn pause_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    state.scheduler.pause_run(&run_id).await.map_err(|e| {
        warn!("failed to pause run {}: {}", id, e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("failed to pause run: {}", e),
            }),
        )
    })?;

    info!("paused run: {}", id);
    Ok(StatusCode::NO_CONTENT)
}

/// POST /runs/{id}/resume - Resume a paused run.
async fn resume_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    let result = state.scheduler.resume_run(&run_id).await.map_err(|e| {
        warn!("failed to resume run {}: {}", id, e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("failed to resume run: {}", e),
            }),
        )
    })?;

    match result {
        Some(_) => {
            info!("resumed run: {}", id);
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "no capacity to resume run".to_string(),
            }),
        )),
    }
}

/// POST /runs/{id}/cancel - Cancel a run.
async fn cancel_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    state.scheduler.cancel_run(&run_id).await.map_err(|e| {
        warn!("failed to cancel run {}: {}", id, e);
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("failed to cancel run: {}", e),
            }),
        )
    })?;

    info!("canceled run: {}", id);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::response::Response;
    use tempfile::TempDir;
    use tower::ServiceExt;

    async fn create_test_app() -> (Router, Arc<AppState>, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        let storage = Arc::new(storage);
        let scheduler = Arc::new(Scheduler::new(Arc::clone(&storage), 3));

        let state = Arc::new(AppState {
            storage,
            scheduler,
            auth_token: None,
        });

        let router = create_router(Arc::clone(&state));
        (router, state, dir)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let (app, _, _dir) = create_test_app().await;

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_run_returns_created() {
        let (app, _, _dir) = create_test_app().await;

        let body = serde_json::json!({
            "spec_path": "/workspace/spec.md",
            "workspace_root": "/workspace"
        });

        let response: Response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/runs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn list_runs_returns_empty_initially() {
        let (app, _, _dir) = create_test_app().await;

        let response: Response = app
            .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_nonexistent_run_returns_404() {
        let (app, _, _dir) = create_test_app().await;

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/runs/nonexistent-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn auth_token_required_when_configured() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        let storage = Arc::new(storage);
        let scheduler = Arc::new(Scheduler::new(Arc::clone(&storage), 3));

        let state = Arc::new(AppState {
            storage,
            scheduler,
            auth_token: Some("secret-token".to_string()),
        });

        let app = create_router(state);

        // Request without token
        let response: Response = app
            .clone()
            .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Request with valid token
        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/runs")
                    .header("authorization", "Bearer secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
