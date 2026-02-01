//! Integration tests for HTTP server and SSE streaming.
//!
//! Tests run lifecycle (create, pause, resume, cancel) and SSE event streaming.
//! Covers spec Section 4.1 and Section 7.1.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use chrono::Utc;
use http_body_util::BodyExt;
use loop_core::events::{EventPayload, RunCreatedPayload, RunStartedPayload, StepFinishedPayload};
use loop_core::{Id, ReviewStatus, Run, RunNameSource, RunStatus, Step, StepPhase, StepStatus};
use loopd::scheduler::Scheduler;
use loopd::server::{create_router, AppState};
use loopd::storage::{Storage, DEFAULT_MAX_CONCURRENT_RUNS};
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

async fn create_test_app() -> (axum::Router, Arc<AppState>, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let storage = Storage::new(&db_path, DEFAULT_MAX_CONCURRENT_RUNS).await.unwrap();
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

async fn body_to_json(response: Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// --- Run Lifecycle Tests ---

#[tokio::test]
async fn run_lifecycle_create_list_get() {
    let (app, _, _dir) = create_test_app().await;

    // Create first run
    let body = serde_json::json!({
        "spec_path": "/workspace/spec1.md",
        "workspace_root": "/workspace",
        "name_source": "spec_slug"
    });

    let response: Response = app
        .clone()
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
    let json = body_to_json(response).await;
    let run1_id = json["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(json["run"]["name"], "spec1");
    assert_eq!(json["run"]["status"], "PENDING");

    // Create second run with explicit name
    let body = serde_json::json!({
        "spec_path": "/workspace/spec2.md",
        "workspace_root": "/workspace",
        "name": "my-custom-run"
    });

    let response: Response = app
        .clone()
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
    let json = body_to_json(response).await;
    let run2_id = json["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(json["run"]["name"], "my-custom-run");

    // List all runs
    let response: Response = app
        .clone()
        .oneshot(Request::builder().uri("/runs").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    assert_eq!(json["runs"].as_array().unwrap().len(), 2);

    // Get specific run
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", run1_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    assert_eq!(json["run"]["id"], run1_id);
    assert_eq!(json["run"]["name"], "spec1");

    // Get second run
    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}", run2_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    assert_eq!(json["run"]["id"], run2_id);
    assert_eq!(json["run"]["name"], "my-custom-run");
}

#[tokio::test]
async fn run_lifecycle_filter_by_workspace() {
    let (app, _, _dir) = create_test_app().await;

    // Create runs in different workspaces
    for (spec, workspace) in [
        ("spec1.md", "/workspace1"),
        ("spec2.md", "/workspace1"),
        ("spec3.md", "/workspace2"),
    ] {
        let body = serde_json::json!({
            "spec_path": format!("{}/{}", workspace, spec),
            "workspace_root": workspace
        });

        let _response: Response = app
            .clone()
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
    }

    // Filter by workspace1
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runs?workspace_root=/workspace1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    assert_eq!(json["runs"].as_array().unwrap().len(), 2);

    // Filter by workspace2
    let response: Response = app
        .oneshot(
            Request::builder()
                .uri("/runs?workspace_root=/workspace2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    assert_eq!(json["runs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn run_lifecycle_pause_resume_cancel() {
    let (_, state, _dir) = create_test_app().await;

    // Create a run directly and set it to running
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Running,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    let app = create_router(Arc::clone(&state));

    // Pause the run
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/runs/{}/pause", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify paused
    let updated = state.storage.get_run(&run_id).await.unwrap();
    assert_eq!(updated.status, RunStatus::Paused);

    // Resume the run
    let app = create_router(Arc::clone(&state));
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/runs/{}/resume", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Resume may succeed or return SERVICE_UNAVAILABLE if no capacity
    assert!(matches!(
        response.status(),
        StatusCode::NO_CONTENT | StatusCode::SERVICE_UNAVAILABLE
    ));

    // Cancel the run (set back to running first for test)
    state
        .storage
        .update_run_status(&run_id, RunStatus::Running)
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));
    let response: Response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/runs/{}/cancel", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify canceled
    let updated = state.storage.get_run(&run_id).await.unwrap();
    assert_eq!(updated.status, RunStatus::Canceled);
}

#[tokio::test]
async fn run_lifecycle_pause_nonexistent_fails() {
    let (app, _, _dir) = create_test_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/runs/nonexistent-id/pause")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn run_lifecycle_list_steps() {
    let (_, state, _dir) = create_test_app().await;

    // Create a run
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Running,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    // Create steps
    for (i, phase) in [
        StepPhase::Implementation,
        StepPhase::Review,
        StepPhase::Verification,
    ]
    .iter()
    .enumerate()
    {
        let step = Step {
            id: Id::new(),
            run_id: run_id.clone(),
            phase: *phase,
            status: if i == 2 {
                StepStatus::InProgress
            } else {
                StepStatus::Succeeded
            },
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: if i < 2 { Some(Utc::now()) } else { None },
            exit_code: if i < 2 { Some(0) } else { None },
            prompt_path: Some(format!("/workspace/logs/loop/prompt-{}.txt", i)),
            output_path: Some(format!("/workspace/logs/loop/output-{}.log", i)),
        };
        state.storage.insert_step(&step).await.unwrap();
    }

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/steps", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_to_json(response).await;
    let steps = json["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0]["phase"], "implementation");
    assert_eq!(steps[1]["phase"], "review");
    assert_eq!(steps[2]["phase"], "verification");
}

// --- SSE Streaming Tests ---

#[tokio::test]
async fn sse_events_returns_correct_content_type() {
    let (_, state, _dir) = create_test_app().await;

    // Create a completed run so stream ends quickly
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/events", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("text/event-stream"));
}

#[tokio::test]
async fn sse_events_streams_historical_events() {
    let (_, state, _dir) = create_test_app().await;

    // Create a completed run
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    // Add events
    let payloads = vec![
        EventPayload::RunCreated(RunCreatedPayload {
            run_id: run_id.clone(),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: None,
        }),
        EventPayload::RunStarted(RunStartedPayload {
            run_id: run_id.clone(),
            worker_id: "worker-1".to_string(),
        }),
    ];

    for payload in &payloads {
        state
            .storage
            .append_event(&run_id, None, payload)
            .await
            .unwrap();
    }

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/events", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Read body and parse SSE events
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);

    // Should contain both events
    assert!(body_str.contains("RUN_CREATED"));
    assert!(body_str.contains("RUN_STARTED"));
}

#[tokio::test]
async fn sse_events_filters_by_after_timestamp() {
    let (_, state, _dir) = create_test_app().await;

    // Create a completed run
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    // Add first event
    let payload1 = EventPayload::RunCreated(RunCreatedPayload {
        run_id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
    });
    state
        .storage
        .append_event(&run_id, None, &payload1)
        .await
        .unwrap();

    // Get timestamp after first event
    let after_ts = Utc::now().timestamp_millis();

    // Small delay to ensure timestamp difference
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Add second event
    let payload2 = EventPayload::RunStarted(RunStartedPayload {
        run_id: run_id.clone(),
        worker_id: "worker-1".to_string(),
    });
    state
        .storage
        .append_event(&run_id, None, &payload2)
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));

    // Request with after filter
    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/events?after={}", run_id, after_ts))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);

    // Should only contain second event
    assert!(!body_str.contains("RUN_CREATED"));
    assert!(body_str.contains("RUN_STARTED"));
}

#[tokio::test]
async fn sse_events_includes_step_events() {
    let (_, state, _dir) = create_test_app().await;

    // Create a completed run with a step
    let run_id = Id::new();
    let step_id = Id::new();

    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    let step = Step {
        id: step_id.clone(),
        run_id: run_id.clone(),
        phase: StepPhase::Implementation,
        status: StepStatus::Succeeded,
        attempt: 1,
        started_at: Some(Utc::now()),
        ended_at: Some(Utc::now()),
        exit_code: Some(0),
        prompt_path: None,
        output_path: Some("/workspace/logs/output.log".to_string()),
    };
    state.storage.insert_step(&step).await.unwrap();

    // Add step event
    let payload = EventPayload::StepFinished(StepFinishedPayload {
        step_id: step_id.clone(),
        exit_code: 0,
        duration_ms: 1000,
        output_path: "/workspace/logs/output.log".to_string(),
    });
    state
        .storage
        .append_event(&run_id, Some(&step_id), &payload)
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/events", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);

    // Should contain step event with step_id
    assert!(body_str.contains("STEP_FINISHED"));
    assert!(body_str.contains(&step_id.to_string()));
}

#[tokio::test]
async fn sse_output_returns_correct_content_type() {
    let (_, state, _dir) = create_test_app().await;

    // Create a completed run
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/output", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("text/event-stream"));
}

#[tokio::test]
async fn sse_output_streams_step_output_content() {
    let (_, state, dir) = create_test_app().await;

    // Create a completed run with a step pointing to a real output file
    let run_id = Id::new();
    let step_id = Id::new();

    let output_path = dir.path().join("output.log");
    std::fs::write(&output_path, "Test output line 1\nTest output line 2\n").unwrap();

    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: dir.path().to_string_lossy().to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    state.storage.insert_run(&run).await.unwrap();

    let step = Step {
        id: step_id.clone(),
        run_id: run_id.clone(),
        phase: StepPhase::Implementation,
        status: StepStatus::Succeeded,
        attempt: 1,
        started_at: Some(Utc::now()),
        ended_at: Some(Utc::now()),
        exit_code: Some(0),
        prompt_path: None,
        output_path: Some(output_path.to_string_lossy().to_string()),
    };
    state.storage.insert_step(&step).await.unwrap();

    let app = create_router(Arc::clone(&state));

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri(format!("/runs/{}/output", run_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);

    // Should contain the output content
    assert!(body_str.contains("Test output line 1"));
    assert!(body_str.contains("Test output line 2"));
}

// --- Auth Token Tests ---

#[tokio::test]
async fn auth_token_blocks_unauthorized_requests() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let storage = Storage::new(&db_path, DEFAULT_MAX_CONCURRENT_RUNS).await.unwrap();
    storage.migrate_embedded().await.unwrap();
    let storage = Arc::new(storage);
    let scheduler = Arc::new(Scheduler::new(Arc::clone(&storage), 3));

    let state = Arc::new(AppState {
        storage: Arc::clone(&storage),
        scheduler,
        auth_token: Some("test-secret-token".to_string()),
    });

    // Create a run for testing
    let run_id = Id::new();
    let run = Run {
        id: run_id.clone(),
        name: "test-run".to_string(),
        name_source: RunNameSource::SpecSlug,
        status: RunStatus::Completed,
        workspace_root: "/workspace".to_string(),
        spec_path: "/workspace/spec.md".to_string(),
        plan_path: None,
        worktree: None,
        worktree_cleanup_status: None,
        worktree_cleaned_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        review_status: ReviewStatus::default(),
        review_action_at: None,
        pr_url: None,
        merge_commit: None,
    };
    storage.insert_run(&run).await.unwrap();

    let app = create_router(Arc::clone(&state));

    // Test various endpoints without token
    let run_uri = format!("/runs/{}", run_id);
    let events_uri = format!("/runs/{}/events", run_id);
    let output_uri = format!("/runs/{}/output", run_id);
    let endpoints = vec!["/runs", &run_uri, &events_uri, &output_uri];

    for uri in endpoints {
        let response: Response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "Expected UNAUTHORIZED for {}",
            uri
        );
    }

    // Test with valid token
    let response: Response = app
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header("authorization", "Bearer test-secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_token_rejects_invalid_token() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let storage = Storage::new(&db_path, DEFAULT_MAX_CONCURRENT_RUNS).await.unwrap();
    storage.migrate_embedded().await.unwrap();
    let storage = Arc::new(storage);
    let scheduler = Arc::new(Scheduler::new(Arc::clone(&storage), 3));

    let state = Arc::new(AppState {
        storage,
        scheduler,
        auth_token: Some("correct-token".to_string()),
    });

    let app = create_router(state);

    let response: Response = app
        .oneshot(
            Request::builder()
                .uri("/runs")
                .header("authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
