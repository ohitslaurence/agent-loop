//! HTTP control plane server for loopd.
//!
//! Implements the local-only REST API from spec Section 4.1.
//! See also Section 8.1 for auth requirements.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use futures_util::{
    stream::{self, Stream},
    StreamExt,
};
use loop_core::{Event, Id, MergeStrategy, Run, RunNameSource, RunStatus};
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
        // SSE streaming endpoints (Section 4.1)
        .route("/runs/{id}/events", get(stream_events))
        .route("/runs/{id}/output", get(stream_output))
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

// --- SSE Streaming Handlers (Section 4.1) ---

/// Query params for GET /runs/{id}/events.
#[derive(Debug, Deserialize, Default)]
pub struct StreamEventsQuery {
    /// Timestamp (ms since epoch) to start from. Events after this time are returned.
    #[serde(default)]
    pub after: Option<i64>,
}

/// SSE event data wrapper for structured events.
#[derive(Debug, Serialize)]
struct SseEventData {
    id: String,
    run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    step_id: Option<String>,
    event_type: String,
    timestamp: i64,
    payload: serde_json::Value,
}

impl From<&Event> for SseEventData {
    fn from(event: &Event) -> Self {
        // Parse the payload JSON to include it as a nested object
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload_json).unwrap_or(serde_json::Value::Null);
        SseEventData {
            id: event.id.to_string(),
            run_id: event.run_id.to_string(),
            step_id: event.step_id.as_ref().map(|id| id.to_string()),
            event_type: event.event_type.clone(),
            timestamp: event.timestamp.timestamp_millis(),
            payload,
        }
    }
}

/// GET /runs/{id}/events - Stream events for a run (SSE).
///
/// Returns a Server-Sent Events stream of structured events.
/// Supports reconnection via `after` query param (timestamp in ms).
async fn stream_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<StreamEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    // Verify run exists
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {}", e),
            }),
        )
    })?;

    let storage = Arc::clone(&state.storage);
    let run_id_clone = run_id.clone();
    let after_ts = query.after;

    // Create a stream that polls for events
    let stream = stream::unfold(
        (storage, run_id_clone, after_ts, run.status, false),
        move |(storage, run_id, last_ts, status, sent_initial)| async move {
            // On first iteration, send all historical events
            if !sent_initial {
                let events = match storage.list_events(&run_id).await {
                    Ok(events) => events,
                    Err(_) => return None,
                };

                // Filter by after timestamp if provided
                let filtered: Vec<_> = events
                    .iter()
                    .filter(|e| {
                        if let Some(after) = last_ts {
                            e.timestamp.timestamp_millis() > after
                        } else {
                            true
                        }
                    })
                    .collect();

                // Get last timestamp for next poll
                let new_last_ts = filtered.last().map(|e| e.timestamp.timestamp_millis());

                // Convert events to SSE events
                let sse_events: Vec<_> = filtered
                    .into_iter()
                    .map(|e| {
                        let data = SseEventData::from(e);
                        let json = serde_json::to_string(&data).unwrap_or_default();
                        Ok(SseEvent::default()
                            .event(&data.event_type)
                            .data(json)
                            .id(data.id))
                    })
                    .collect();

                let events_stream = stream::iter(sse_events);
                return Some((
                    events_stream,
                    (storage, run_id, new_last_ts.or(last_ts), status, true),
                ));
            }

            // Check if run is terminal (completed, failed, canceled)
            let current_run = match storage.get_run(&run_id).await {
                Ok(r) => r,
                Err(_) => return None,
            };

            let is_terminal = matches!(
                current_run.status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::Canceled
            );

            // If terminal and we've already sent initial events, check for new events once more
            if is_terminal {
                // Get any new events since last check
                let events = match storage.list_events(&run_id).await {
                    Ok(events) => events,
                    Err(_) => return None,
                };

                let filtered: Vec<_> = events
                    .iter()
                    .filter(|e| {
                        if let Some(after) = last_ts {
                            e.timestamp.timestamp_millis() > after
                        } else {
                            false // Already sent all in initial batch
                        }
                    })
                    .collect();

                if !filtered.is_empty() {
                    let sse_events: Vec<_> = filtered
                        .into_iter()
                        .map(|e| {
                            let data = SseEventData::from(e);
                            let json = serde_json::to_string(&data).unwrap_or_default();
                            Ok(SseEvent::default()
                                .event(&data.event_type)
                                .data(json)
                                .id(data.id))
                        })
                        .collect();
                    return Some((
                        stream::iter(sse_events),
                        (storage, run_id, None, current_run.status, true),
                    ));
                }
                // Terminal and no more events, end stream
                return None;
            }

            // Non-terminal: poll for new events after a delay
            tokio::time::sleep(Duration::from_secs(1)).await;

            let events = match storage.list_events(&run_id).await {
                Ok(events) => events,
                Err(_) => {
                    return Some((
                        stream::iter(vec![]),
                        (storage, run_id, last_ts, current_run.status, true),
                    ))
                }
            };

            let filtered: Vec<_> = events
                .iter()
                .filter(|e| {
                    if let Some(after) = last_ts {
                        e.timestamp.timestamp_millis() > after
                    } else {
                        false // Already sent all in initial batch
                    }
                })
                .collect();

            let new_last_ts = filtered.last().map(|e| e.timestamp.timestamp_millis());

            let sse_events: Vec<_> = filtered
                .into_iter()
                .map(|e| {
                    let data = SseEventData::from(e);
                    let json = serde_json::to_string(&data).unwrap_or_default();
                    Ok(SseEvent::default()
                        .event(&data.event_type)
                        .data(json)
                        .id(data.id))
                })
                .collect();

            Some((
                stream::iter(sse_events),
                (
                    storage,
                    run_id,
                    new_last_ts.or(last_ts),
                    current_run.status,
                    true,
                ),
            ))
        },
    )
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Query params for GET /runs/{id}/output.
#[derive(Debug, Deserialize, Default)]
pub struct StreamOutputQuery {
    /// Byte offset to start reading from.
    #[serde(default)]
    pub offset: Option<u64>,
}

/// GET /runs/{id}/output - Stream raw iteration output (SSE).
///
/// Returns a Server-Sent Events stream of raw output chunks.
/// Reads from step output files and streams new content as it appears.
async fn stream_output(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<StreamOutputQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    check_auth(&state, &headers)?;

    let run_id = Id::from_string(&id);

    // Verify run exists
    let run = state.storage.get_run(&run_id).await.map_err(|e| {
        warn!("run not found: {}", id);
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("run not found: {}", e),
            }),
        )
    })?;

    let storage = Arc::clone(&state.storage);
    let run_id_clone = run_id.clone();
    let initial_offset = query.offset.unwrap_or(0);

    // Stream state: (storage, run_id, current_step_idx, current_file_offset, run_status)
    let stream = stream::unfold(
        (storage, run_id_clone, 0usize, initial_offset, run.status),
        move |(storage, run_id, step_idx, file_offset, _status)| async move {
            loop {
                // Get current run status
                let current_run = match storage.get_run(&run_id).await {
                    Ok(r) => r,
                    Err(_) => return None,
                };

                // Get all steps
                let steps = match storage.list_steps(&run_id).await {
                    Ok(s) => s,
                    Err(_) => return None,
                };

                // If no steps yet, wait and retry
                if steps.is_empty() {
                    if matches!(
                        current_run.status,
                        RunStatus::Completed | RunStatus::Failed | RunStatus::Canceled
                    ) {
                        return None; // Run ended with no steps
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }

                // Process steps starting from step_idx
                for (idx, step) in steps.iter().enumerate().skip(step_idx) {
                    if let Some(output_path) = &step.output_path {
                        // Try to read output file
                        let path = std::path::Path::new(output_path);
                        if path.exists() {
                            let metadata = match std::fs::metadata(path) {
                                Ok(m) => m,
                                Err(_) => continue,
                            };

                            let current_offset = if idx == step_idx { file_offset } else { 0 };
                            let file_size = metadata.len();

                            if file_size > current_offset {
                                // Read new content
                                let content = match std::fs::read_to_string(path) {
                                    Ok(c) => c,
                                    Err(_) => continue,
                                };

                                let new_content = &content[current_offset as usize..];
                                if !new_content.is_empty() {
                                    let data = serde_json::json!({
                                        "step_id": step.id.to_string(),
                                        "offset": current_offset,
                                        "content": new_content,
                                    });
                                    let event = Ok(SseEvent::default()
                                        .event("output")
                                        .data(serde_json::to_string(&data).unwrap_or_default()));

                                    return Some((
                                        event,
                                        (storage, run_id, idx, file_size, current_run.status),
                                    ));
                                }
                            }
                        }
                    }
                }

                // Check if run is terminal
                if matches!(
                    current_run.status,
                    RunStatus::Completed | RunStatus::Failed | RunStatus::Canceled
                ) {
                    return None; // End of stream
                }

                // No new content, wait and retry
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::response::Response;
    use loop_core::events::{EventPayload, RunCreatedPayload};
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

    #[tokio::test]
    async fn stream_events_returns_sse_for_existing_run() {
        let (app, state, _dir) = create_test_app().await;

        // Create a run
        let run = Run {
            id: Id::new(),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Completed, // Terminal so stream ends
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        state.storage.insert_run(&run).await.unwrap();

        // Add an event
        let payload = EventPayload::RunCreated(RunCreatedPayload {
            run_id: run.id.clone(),
            name: run.name.clone(),
            name_source: run.name_source,
            spec_path: run.spec_path.clone(),
            plan_path: run.plan_path.clone(),
        });
        state
            .storage
            .append_event(&run.id, None, &payload)
            .await
            .unwrap();

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/runs/{}/events", run.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap_or("")),
            Some("text/event-stream")
        );
    }

    #[tokio::test]
    async fn stream_events_returns_404_for_nonexistent_run() {
        let (app, _, _dir) = create_test_app().await;

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/runs/nonexistent-id/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stream_output_returns_404_for_nonexistent_run() {
        let (app, _, _dir) = create_test_app().await;

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/runs/nonexistent-id/output")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stream_output_returns_sse_for_completed_run() {
        let (app, state, _dir) = create_test_app().await;

        // Create a completed run (terminal so stream ends immediately)
        let run = Run {
            id: Id::new(),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Completed,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        state.storage.insert_run(&run).await.unwrap();

        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/runs/{}/output", run.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .map(|v| v.to_str().unwrap_or("")),
            Some("text/event-stream")
        );
    }
}
