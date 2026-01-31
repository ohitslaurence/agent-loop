//! SQLite storage module for the orchestrator daemon.
//!
//! Implements persistence for runs, steps, events, and artifacts.
//! See spec Section 3.2 and Section 4.2.

use chrono::{DateTime, Utc};
use loop_core::{
    events::EventPayload, Artifact, ArtifactLocation, Config, Event, Id, MergeStrategy, Run,
    RunNameSource, RunStatus, RunWorktree, Step, StepPhase, StepStatus, WorktreeProvider,
};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;
use thiserror::Error;

/// Explicit column list for runs table queries.
/// Using explicit columns instead of SELECT * ensures correct mapping
/// regardless of column order in the database (important for ALTER TABLE migrations).
const RUNS_COLUMNS: &str = "id, name, name_source, status, workspace_root, spec_path, \
    plan_path, base_branch, run_branch, merge_target_branch, merge_strategy, \
    worktree_path, config_json, created_at, updated_at, worktree_provider, \
    worktree_cleanup_status, worktree_cleaned_at";

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(String),
    #[error("run not found: {0}")]
    RunNotFound(String),
    #[error("step not found: {0}")]
    StepNotFound(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Storage backend for the daemon.
pub struct Storage {
    pool: Pool<Sqlite>,
}

impl Storage {
    /// Create a new storage instance with the given database path.
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        // Enable WAL mode
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Run migrations to initialize/update the schema.
    pub async fn migrate(&self, migrations_path: &Path) -> Result<()> {
        let migrator = sqlx::migrate::Migrator::new(migrations_path).await?;
        migrator.run(&self.pool).await?;
        Ok(())
    }

    /// Run embedded migrations (for when migrations are compiled in).
    pub async fn migrate_embedded(&self) -> Result<()> {
        // Run all embedded migrations in order.
        let migrations = [
            include_str!("../../../migrations/0001_init.sql"),
            include_str!("../../../migrations/0002_add_worktree_provider.sql"),
            include_str!("../../../migrations/0003_add_worktree_cleanup_state.sql"),
        ];

        for migration_sql in migrations {
            // Remove comment lines before splitting.
            let cleaned: String = migration_sql
                .lines()
                .filter(|line| !line.trim().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");

            for statement in cleaned.split(';') {
                let trimmed = statement.trim();
                if !trimmed.is_empty() {
                    match sqlx::query(trimmed).execute(&self.pool).await {
                        Ok(_) => {}
                        Err(e) => {
                            let msg = e.to_string();
                            // Ignore expected idempotent errors (duplicate column, table exists).
                            if !msg.contains("duplicate column") && !msg.contains("already exists")
                            {
                                return Err(e.into());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // --- Run operations ---

    /// Insert a new run.
    pub async fn insert_run(&self, run: &Run) -> Result<()> {
        let name_source = run.name_source.as_str();
        let status = run.status.as_str();
        let (
            base_branch,
            run_branch,
            merge_target,
            merge_strategy,
            worktree_path,
            worktree_provider,
        ) = match &run.worktree {
            Some(wt) => (
                Some(wt.base_branch.as_str()),
                Some(wt.run_branch.as_str()),
                wt.merge_target_branch.as_deref(),
                Some(wt.merge_strategy.as_str()),
                Some(wt.worktree_path.as_str()),
                Some(wt.provider.as_str()),
            ),
            None => (None, None, None, None, None, None),
        };
        let created_at = run.created_at.timestamp_millis();
        let updated_at = run.updated_at.timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO runs (id, name, name_source, status, workspace_root, spec_path, plan_path,
                              base_branch, run_branch, merge_target_branch, merge_strategy,
                              worktree_path, worktree_provider, config_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(run.id.as_ref())
        .bind(&run.name)
        .bind(name_source)
        .bind(status)
        .bind(&run.workspace_root)
        .bind(&run.spec_path)
        .bind(&run.plan_path)
        .bind(base_branch)
        .bind(run_branch)
        .bind(merge_target)
        .bind(merge_strategy)
        .bind(worktree_path)
        .bind(worktree_provider)
        .bind(&run.config_json)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a run by ID.
    pub async fn get_run(&self, id: &Id) -> Result<Run> {
        let query = format!("SELECT {} FROM runs WHERE id = ?1", RUNS_COLUMNS);
        let row = sqlx::query_as::<_, RunRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StorageError::RunNotFound(id.to_string()))?;

        Ok(row.into_run())
    }

    /// List runs, optionally filtered by workspace.
    pub async fn list_runs(&self, workspace_root: Option<&str>) -> Result<Vec<Run>> {
        let rows = match workspace_root {
            Some(ws) => {
                let query = format!(
                    "SELECT {} FROM runs WHERE workspace_root = ?1 ORDER BY created_at DESC",
                    RUNS_COLUMNS
                );
                sqlx::query_as::<_, RunRow>(&query)
                    .bind(ws)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                let query = format!("SELECT {} FROM runs ORDER BY created_at DESC", RUNS_COLUMNS);
                sqlx::query_as::<_, RunRow>(&query)
                    .fetch_all(&self.pool)
                    .await?
            }
        };

        Ok(rows.into_iter().map(|r| r.into_run()).collect())
    }

    /// Count running runs for a specific workspace.
    ///
    /// Used for per-workspace cap enforcement (spec Section 4.2, 5.3).
    pub async fn count_running_runs_for_workspace(&self, workspace_root: &str) -> Result<usize> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM runs WHERE workspace_root = ?1 AND status = 'RUNNING'",
        )
        .bind(workspace_root)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.0 as usize)
    }

    /// Update run status.
    pub async fn update_run_status(&self, id: &Id, status: RunStatus) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let result = sqlx::query("UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(status.as_str())
            .bind(now)
            .bind(id.as_ref())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::RunNotFound(id.to_string()));
        }
        Ok(())
    }

    /// Update worktree fields for a run.
    pub async fn update_run_worktree(&self, id: &Id, worktree: &RunWorktree) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE runs SET base_branch = ?1, run_branch = ?2, merge_target_branch = ?3, \
             merge_strategy = ?4, worktree_path = ?5, worktree_provider = ?6, updated_at = ?7 \
             WHERE id = ?8",
        )
        .bind(&worktree.base_branch)
        .bind(&worktree.run_branch)
        .bind(&worktree.merge_target_branch)
        .bind(worktree.merge_strategy.as_str())
        .bind(&worktree.worktree_path)
        .bind(worktree.provider.as_str())
        .bind(now)
        .bind(id.as_ref())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::RunNotFound(id.to_string()));
        }

        Ok(())
    }

    /// Update worktree cleanup status for a run.
    pub async fn update_run_worktree_cleanup(
        &self,
        id: &Id,
        status: &str,
        cleaned_at: Option<i64>,
    ) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE runs SET worktree_cleanup_status = ?1, worktree_cleaned_at = ?2, \
             updated_at = ?3 WHERE id = ?4",
        )
        .bind(status)
        .bind(cleaned_at)
        .bind(now)
        .bind(id.as_ref())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::RunNotFound(id.to_string()));
        }

        Ok(())
    }

    // --- Step operations ---

    /// Insert a new step.
    pub async fn insert_step(&self, step: &Step) -> Result<()> {
        let phase = step.phase.as_str();
        let status = step.status.as_str();
        let started_at = step.started_at.map(|t| t.timestamp_millis());
        let ended_at = step.ended_at.map(|t| t.timestamp_millis());

        sqlx::query(
            r#"
            INSERT INTO steps (id, run_id, phase, status, attempt, started_at, ended_at,
                               exit_code, prompt_path, output_path)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(step.id.as_ref())
        .bind(step.run_id.as_ref())
        .bind(phase)
        .bind(status)
        .bind(step.attempt as i64)
        .bind(started_at)
        .bind(ended_at)
        .bind(step.exit_code)
        .bind(&step.prompt_path)
        .bind(&step.output_path)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a step by ID.
    pub async fn get_step(&self, id: &Id) -> Result<Step> {
        let row = sqlx::query_as::<_, StepRow>("SELECT * FROM steps WHERE id = ?1")
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StorageError::StepNotFound(id.to_string()))?;

        Ok(row.into_step())
    }

    /// List steps for a run.
    pub async fn list_steps(&self, run_id: &Id) -> Result<Vec<Step>> {
        let rows = sqlx::query_as::<_, StepRow>(
            "SELECT * FROM steps WHERE run_id = ?1 ORDER BY started_at ASC",
        )
        .bind(run_id.as_ref())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_step()).collect())
    }

    /// Update step status and timing.
    pub async fn update_step(
        &self,
        id: &Id,
        status: StepStatus,
        exit_code: Option<i32>,
        output_path: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE steps SET status = ?1, ended_at = ?2, exit_code = ?3, output_path = ?4 WHERE id = ?5",
        )
        .bind(status.as_str())
        .bind(now)
        .bind(exit_code)
        .bind(output_path)
        .bind(id.as_ref())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(StorageError::StepNotFound(id.to_string()));
        }
        Ok(())
    }

    // --- Event operations ---

    /// Append an event to the audit log.
    pub async fn append_event(
        &self,
        run_id: &Id,
        step_id: Option<&Id>,
        payload: &EventPayload,
    ) -> Result<Event> {
        let id = Id::new();
        let now = Utc::now();
        let event_type = payload.event_type().as_str().to_string();
        let payload_json = payload.to_json()?;

        sqlx::query(
            "INSERT INTO events (id, run_id, step_id, type, ts, payload_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id.as_ref())
        .bind(run_id.as_ref())
        .bind(step_id.map(|s| s.as_ref()))
        .bind(&event_type)
        .bind(now.timestamp_millis())
        .bind(&payload_json)
        .execute(&self.pool)
        .await?;

        Ok(Event {
            id,
            run_id: run_id.clone(),
            step_id: step_id.cloned(),
            event_type,
            timestamp: now,
            payload_json,
        })
    }

    /// List events for a run.
    pub async fn list_events(&self, run_id: &Id) -> Result<Vec<Event>> {
        let rows =
            sqlx::query_as::<_, EventRow>("SELECT * FROM events WHERE run_id = ?1 ORDER BY ts ASC")
                .bind(run_id.as_ref())
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|r| r.into_event()).collect())
    }

    // --- Artifact operations ---

    /// Insert an artifact reference.
    pub async fn insert_artifact(&self, artifact: &Artifact) -> Result<()> {
        sqlx::query(
            "INSERT INTO artifacts (id, run_id, kind, location, path, checksum) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(artifact.id.as_ref())
        .bind(artifact.run_id.as_ref())
        .bind(&artifact.kind)
        .bind(artifact.location.as_str())
        .bind(&artifact.path)
        .bind(&artifact.checksum)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// List artifacts for a run.
    pub async fn list_artifacts(&self, run_id: &Id) -> Result<Vec<Artifact>> {
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT * FROM artifacts WHERE run_id = ?1 ORDER BY kind",
        )
        .bind(run_id.as_ref())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_artifact()).collect())
    }

    // --- Report TSV export (Section 7.1) ---

    /// Export events for a run to report.tsv format.
    ///
    /// This generates a TSV file compatible with `bin/loop-analyze`.
    pub async fn export_report(&self, run_id: &Id, report_path: &Path) -> Result<()> {
        let run = self.get_run(run_id).await?;
        let events = self.list_events(run_id).await?;
        let steps = self.list_steps(run_id).await?;

        let rows = events_to_report_rows(&run, &events, &steps);
        loop_core::report::write_report(report_path, &rows)
            .map_err(|e| StorageError::Io(e.to_string()))?;

        Ok(())
    }
}

/// Convert events and steps to report TSV rows.
fn events_to_report_rows(run: &Run, events: &[Event], steps: &[Step]) -> Vec<loop_core::ReportRow> {
    use loop_core::ReportRow;

    let mut rows = Vec::new();
    let run_config = parse_run_config(run);

    // Build a map of step_id -> step for quick lookup.
    let step_map: std::collections::HashMap<&str, &Step> =
        steps.iter().map(|s| (s.id.as_ref(), s)).collect();

    for event in events {
        let ts = event.timestamp.timestamp_millis();

        // Map daemon event types to bin/loop report kinds.
        match event.event_type.as_str() {
            "RUN_CREATED" => {
                // Build message similar to bin/loop RUN_START.
                let message = format!(
                    "spec={} plan={} iterations={} model={} mode=plan",
                    run.spec_path,
                    run.plan_path.as_deref().unwrap_or(""),
                    run_config.iterations,
                    run_config.model,
                );
                rows.push(ReportRow::new(ts, "RUN_START").with_message(message));
            }
            "RUN_STARTED" => {
                // RUN_STARTED is internal; we already emit RUN_START from RUN_CREATED.
            }
            "STEP_STARTED" => {
                if let Some(step_id) = &event.step_id {
                    if let Some(step) = step_map.get(step_id.as_ref()) {
                        let iter_label = format_iteration_label(step);
                        rows.push(
                            ReportRow::new(ts, "ITERATION_START").with_iteration(&iter_label),
                        );
                    }
                }
            }
            "STEP_FINISHED" => {
                if let Some(step_id) = &event.step_id {
                    if let Some(step) = step_map.get(step_id.as_ref()) {
                        let iter_label = format_iteration_label(step);
                        let mut row =
                            ReportRow::new(ts, "ITERATION_END").with_iteration(&iter_label);

                        // Extract data from step.
                        if let (Some(start), Some(end)) = (step.started_at, step.ended_at) {
                            let duration = (end - start).num_milliseconds() as u64;
                            row = row.with_duration_ms(duration);
                        }
                        if let Some(code) = step.exit_code {
                            row = row.with_exit_code(code);
                        }
                        if let Some(ref path) = step.output_path {
                            row = row.with_output_path(path);
                            // Try to get file stats.
                            if let Ok(meta) = std::fs::metadata(path) {
                                row = row.with_output(meta.len(), count_lines(path));
                            }
                        }

                        rows.push(row);
                    }
                }
            }
            "RUN_COMPLETED" => {
                // Extract mode from payload if possible.
                let mode = extract_completion_mode(&event.payload_json);
                let message = format!("mode={}", mode);
                rows.push(ReportRow::new(ts, "COMPLETE_DETECTED").with_message(message));
                rows.push(ReportRow::new(ts, "RUN_END").with_message("reason=complete"));
            }
            "RUN_FAILED" => {
                let reason = extract_failure_reason(&event.payload_json);
                rows.push(
                    ReportRow::new(ts, "RUN_END").with_message(format!("reason=failed:{}", reason)),
                );
            }
            "WATCHDOG_REWRITE" => {
                if let Some(step_id) = &event.step_id {
                    if let Some(step) = step_map.get(step_id.as_ref()) {
                        let iter_label = format_iteration_label(step);
                        let signal = extract_watchdog_signal(&event.payload_json);
                        rows.push(
                            ReportRow::new(ts, "WATCHDOG_REWRITE")
                                .with_iteration(&iter_label)
                                .with_message(format!("signal={}", signal)),
                        );
                    }
                }
            }
            _ => {
                // Unknown event type; skip or log.
            }
        }
    }

    rows
}

/// Format iteration label from step (e.g., "1", "1R1", "2").
fn format_iteration_label(step: &Step) -> String {
    match step.phase {
        StepPhase::Review => format!("{}R{}", step.attempt, 1), // Review iterations
        _ => step.attempt.to_string(),
    }
}

/// Count lines in a file.
fn count_lines(path: &str) -> u64 {
    std::fs::read_to_string(path)
        .map(|s| s.lines().count() as u64)
        .unwrap_or(0)
}

fn parse_run_config(run: &Run) -> Config {
    if let Some(config_json) = run.config_json.as_ref() {
        if let Ok(config) = serde_json::from_str::<Config>(config_json) {
            return config;
        }

        let path = Path::new(config_json);
        if path.exists() {
            if let Ok(config) = Config::from_file(path) {
                return config;
            }
        }
    }

    Config::default()
}

/// Extract completion mode from RUN_COMPLETED payload.
fn extract_completion_mode(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| "trailing".to_string())
}

/// Extract failure reason from RUN_FAILED payload.
fn extract_failure_reason(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Extract watchdog signal from WATCHDOG_REWRITE payload.
fn extract_watchdog_signal(payload: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("signal").and_then(|s| s.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".to_string())
}

// --- Row types for SQLx ---

#[derive(sqlx::FromRow)]
struct RunRow {
    id: String,
    name: String,
    name_source: String,
    status: String,
    workspace_root: String,
    spec_path: String,
    plan_path: Option<String>,
    base_branch: Option<String>,
    run_branch: Option<String>,
    merge_target_branch: Option<String>,
    merge_strategy: Option<String>,
    worktree_path: Option<String>,
    config_json: Option<String>,
    created_at: i64,
    updated_at: i64,
    // NOTE: worktree_provider is at the end because ALTER TABLE adds columns at the end.
    // The struct field order must match the database column order for SELECT *.
    worktree_provider: Option<String>,
    worktree_cleanup_status: Option<String>,
    worktree_cleaned_at: Option<i64>,
}

impl RunRow {
    fn into_run(self) -> Run {
        let name_source = match self.name_source.as_str() {
            "spec_slug" => RunNameSource::SpecSlug,
            _ => RunNameSource::Haiku,
        };
        let status = match self.status.as_str() {
            "PENDING" => RunStatus::Pending,
            "RUNNING" => RunStatus::Running,
            "PAUSED" => RunStatus::Paused,
            "COMPLETED" => RunStatus::Completed,
            "FAILED" => RunStatus::Failed,
            "CANCELED" => RunStatus::Canceled,
            _ => RunStatus::Failed,
        };
        let worktree = match (self.base_branch, self.run_branch, self.worktree_path) {
            (Some(base), Some(run_br), Some(wt_path)) => Some(RunWorktree {
                base_branch: base,
                run_branch: run_br,
                merge_target_branch: self.merge_target_branch,
                merge_strategy: match self.merge_strategy.as_deref() {
                    Some("none") => MergeStrategy::None,
                    Some("merge") => MergeStrategy::Merge,
                    _ => MergeStrategy::Squash,
                },
                worktree_path: wt_path,
                provider: match self.worktree_provider.as_deref() {
                    Some("worktrunk") => WorktreeProvider::Worktrunk,
                    Some("git") => WorktreeProvider::Git,
                    _ => WorktreeProvider::Auto,
                },
            }),
            _ => None,
        };

        Run {
            id: Id::from_string(self.id),
            name: self.name,
            name_source,
            status,
            workspace_root: self.workspace_root,
            spec_path: self.spec_path,
            plan_path: self.plan_path,
            worktree,
            worktree_cleanup_status: self.worktree_cleanup_status,
            worktree_cleaned_at: self
                .worktree_cleaned_at
                .and_then(DateTime::from_timestamp_millis),
            config_json: self.config_json,
            created_at: DateTime::from_timestamp_millis(self.created_at).unwrap_or_default(),
            updated_at: DateTime::from_timestamp_millis(self.updated_at).unwrap_or_default(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct StepRow {
    id: String,
    run_id: String,
    phase: String,
    status: String,
    attempt: i64,
    started_at: Option<i64>,
    ended_at: Option<i64>,
    exit_code: Option<i32>,
    prompt_path: Option<String>,
    output_path: Option<String>,
}

impl StepRow {
    fn into_step(self) -> Step {
        let phase = match self.phase.as_str() {
            "implementation" => StepPhase::Implementation,
            "review" => StepPhase::Review,
            "verification" => StepPhase::Verification,
            "watchdog" => StepPhase::Watchdog,
            "merge" => StepPhase::Merge,
            _ => StepPhase::Implementation,
        };
        let status = match self.status.as_str() {
            "QUEUED" => StepStatus::Queued,
            "IN_PROGRESS" => StepStatus::InProgress,
            "SUCCEEDED" => StepStatus::Succeeded,
            "FAILED" => StepStatus::Failed,
            "RETRYING" => StepStatus::Retrying,
            "CANCELED" => StepStatus::Canceled,
            _ => StepStatus::Failed,
        };

        Step {
            id: Id::from_string(self.id),
            run_id: Id::from_string(self.run_id),
            phase,
            status,
            attempt: self.attempt as u32,
            started_at: self.started_at.and_then(DateTime::from_timestamp_millis),
            ended_at: self.ended_at.and_then(DateTime::from_timestamp_millis),
            exit_code: self.exit_code,
            prompt_path: self.prompt_path,
            output_path: self.output_path,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    id: String,
    run_id: String,
    step_id: Option<String>,
    #[sqlx(rename = "type")]
    event_type: String,
    ts: i64,
    payload_json: String,
}

impl EventRow {
    fn into_event(self) -> Event {
        Event {
            id: Id::from_string(self.id),
            run_id: Id::from_string(self.run_id),
            step_id: self.step_id.map(Id::from_string),
            event_type: self.event_type,
            timestamp: DateTime::from_timestamp_millis(self.ts).unwrap_or_default(),
            payload_json: self.payload_json,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    id: String,
    run_id: String,
    kind: String,
    location: String,
    path: String,
    checksum: Option<String>,
}

impl ArtifactRow {
    fn into_artifact(self) -> Artifact {
        let location = match self.location.as_str() {
            "workspace" => ArtifactLocation::Workspace,
            "global" => ArtifactLocation::Global,
            _ => ArtifactLocation::Workspace,
        };

        Artifact {
            id: Id::from_string(self.id),
            run_id: Id::from_string(self.run_id),
            kind: self.kind,
            location,
            path: self.path,
            checksum: self.checksum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loop_core::events::RunCreatedPayload;
    use tempfile::TempDir;

    struct TestStorage {
        storage: Storage,
        _dir: TempDir, // Keep alive to prevent cleanup
    }

    async fn create_test_storage() -> TestStorage {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();
        storage.migrate_embedded().await.unwrap();
        TestStorage { storage, _dir: dir }
    }

    fn create_test_run() -> Run {
        let now = Utc::now();
        Run {
            id: Id::new(),
            name: "test-run".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn insert_and_get_run() {
        let ts = create_test_storage().await;
        let run = create_test_run();

        ts.storage.insert_run(&run).await.unwrap();
        let retrieved = ts.storage.get_run(&run.id).await.unwrap();

        assert_eq!(retrieved.id, run.id);
        assert_eq!(retrieved.name, run.name);
        assert_eq!(retrieved.status, RunStatus::Pending);
    }

    #[tokio::test]
    async fn update_run_status() {
        let ts = create_test_storage().await;
        let run = create_test_run();

        ts.storage.insert_run(&run).await.unwrap();
        ts.storage
            .update_run_status(&run.id, RunStatus::Running)
            .await
            .unwrap();

        let retrieved = ts.storage.get_run(&run.id).await.unwrap();
        assert_eq!(retrieved.status, RunStatus::Running);
    }

    #[tokio::test]
    async fn update_run_worktree_cleanup_updates_fields() {
        let ts = create_test_storage().await;
        let run = create_test_run();

        ts.storage.insert_run(&run).await.unwrap();
        let cleaned_at = Utc::now().timestamp_millis();
        ts.storage
            .update_run_worktree_cleanup(&run.id, "cleaned", Some(cleaned_at))
            .await
            .unwrap();

        let updated = ts.storage.get_run(&run.id).await.unwrap();
        assert_eq!(updated.worktree_cleanup_status.as_deref(), Some("cleaned"));
        assert_eq!(
            updated.worktree_cleaned_at.map(|t| t.timestamp_millis()),
            Some(cleaned_at)
        );
    }

    #[tokio::test]
    async fn insert_and_list_steps() {
        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        let step = Step {
            id: Id::new(),
            run_id: run.id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::Queued,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        };

        ts.storage.insert_step(&step).await.unwrap();
        let steps = ts.storage.list_steps(&run.id).await.unwrap();

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].phase, StepPhase::Implementation);
    }

    #[tokio::test]
    async fn append_and_list_events() {
        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        let payload = EventPayload::RunCreated(RunCreatedPayload {
            run_id: run.id.clone(),
            name: run.name.clone(),
            name_source: run.name_source,
            spec_path: run.spec_path.clone(),
            plan_path: run.plan_path.clone(),
        });

        ts.storage
            .append_event(&run.id, None, &payload)
            .await
            .unwrap();
        let events = ts.storage.list_events(&run.id).await.unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "RUN_CREATED");
    }

    #[tokio::test]
    async fn insert_and_list_artifacts() {
        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        let artifact = Artifact {
            id: Id::new(),
            run_id: run.id.clone(),
            kind: "prompt".to_string(),
            location: ArtifactLocation::Workspace,
            path: "/workspace/logs/prompt.txt".to_string(),
            checksum: None,
        };

        ts.storage.insert_artifact(&artifact).await.unwrap();
        let artifacts = ts.storage.list_artifacts(&run.id).await.unwrap();

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, "prompt");
    }

    #[tokio::test]
    async fn export_report_generates_tsv() {
        use loop_core::events::{RunCreatedPayload, StepFinishedPayload, StepStartedPayload};

        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        // Add RUN_CREATED event.
        let payload = EventPayload::RunCreated(RunCreatedPayload {
            run_id: run.id.clone(),
            name: run.name.clone(),
            name_source: run.name_source,
            spec_path: run.spec_path.clone(),
            plan_path: run.plan_path.clone(),
        });
        ts.storage
            .append_event(&run.id, None, &payload)
            .await
            .unwrap();

        // Add step and events.
        let step = Step {
            id: Id::new(),
            run_id: run.id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::Succeeded,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: Some(Utc::now() + chrono::Duration::seconds(60)),
            exit_code: Some(0),
            prompt_path: None,
            output_path: None,
        };
        ts.storage.insert_step(&step).await.unwrap();

        // STEP_STARTED event.
        let start_payload = EventPayload::StepStarted(StepStartedPayload {
            step_id: step.id.clone(),
            phase: "implementation".to_string(),
            attempt: 1,
        });
        ts.storage
            .append_event(&run.id, Some(&step.id), &start_payload)
            .await
            .unwrap();

        // STEP_FINISHED event.
        let finish_payload = EventPayload::StepFinished(StepFinishedPayload {
            step_id: step.id.clone(),
            exit_code: 0,
            duration_ms: 60000,
            output_path: "/test/output.log".to_string(),
        });
        ts.storage
            .append_event(&run.id, Some(&step.id), &finish_payload)
            .await
            .unwrap();

        // Export report.
        let report_path = ts._dir.path().join("report.tsv");
        ts.storage
            .export_report(&run.id, &report_path)
            .await
            .unwrap();

        // Verify file was created and contains expected content.
        let content = std::fs::read_to_string(&report_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // Header + at least RUN_START, ITERATION_START, ITERATION_END.
        assert!(
            lines.len() >= 4,
            "Expected at least 4 lines, got {}",
            lines.len()
        );
        assert!(lines[0].contains("timestamp_ms"), "Header missing");
        assert!(lines[1].contains("RUN_START"), "RUN_START missing");
        assert!(
            lines[2].contains("ITERATION_START"),
            "ITERATION_START missing"
        );
        assert!(lines[3].contains("ITERATION_END"), "ITERATION_END missing");
    }

    #[tokio::test]
    async fn migrate_embedded_creates_tables() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();

        // Should succeed without error.
        storage.migrate_embedded().await.unwrap();

        // Verify tables exist by inserting a run.
        let run = create_test_run();
        storage.insert_run(&run).await.unwrap();

        // Verify it can be retrieved.
        let retrieved = storage.get_run(&run.id).await.unwrap();
        assert_eq!(retrieved.id, run.id);
    }

    #[tokio::test]
    async fn migrate_embedded_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).await.unwrap();

        // Run migrations twice - should not error.
        storage.migrate_embedded().await.unwrap();
        storage.migrate_embedded().await.unwrap();

        // Should still work.
        let run = create_test_run();
        storage.insert_run(&run).await.unwrap();
    }

    #[tokio::test]
    async fn get_run_not_found() {
        let ts = create_test_storage().await;
        let missing_id = Id::new();

        let result = ts.storage.get_run(&missing_id).await;
        assert!(matches!(result, Err(StorageError::RunNotFound(_))));
    }

    #[tokio::test]
    async fn get_step_not_found() {
        let ts = create_test_storage().await;
        let missing_id = Id::new();

        let result = ts.storage.get_step(&missing_id).await;
        assert!(matches!(result, Err(StorageError::StepNotFound(_))));
    }

    #[tokio::test]
    async fn update_run_status_not_found() {
        let ts = create_test_storage().await;
        let missing_id = Id::new();

        let result = ts
            .storage
            .update_run_status(&missing_id, RunStatus::Running)
            .await;
        assert!(matches!(result, Err(StorageError::RunNotFound(_))));
    }

    #[tokio::test]
    async fn update_step_not_found() {
        let ts = create_test_storage().await;
        let missing_id = Id::new();

        let result = ts
            .storage
            .update_step(&missing_id, StepStatus::Succeeded, Some(0), None)
            .await;
        assert!(matches!(result, Err(StorageError::StepNotFound(_))));
    }

    #[tokio::test]
    async fn list_runs_filters_by_workspace() {
        let ts = create_test_storage().await;

        // Create runs in different workspaces.
        let now = Utc::now();
        let run1 = Run {
            id: Id::new(),
            name: "run1".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace-a".to_string(),
            spec_path: "/workspace-a/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let run2 = Run {
            id: Id::new(),
            name: "run2".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace-b".to_string(),
            spec_path: "/workspace-b/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };

        ts.storage.insert_run(&run1).await.unwrap();
        ts.storage.insert_run(&run2).await.unwrap();

        // Filter by workspace-a.
        let filtered = ts.storage.list_runs(Some("/workspace-a")).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "run1");

        // No filter returns all.
        let all = ts.storage.list_runs(None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn run_worktree_fields_round_trip() {
        let ts = create_test_storage().await;
        let now = Utc::now();

        let run = Run {
            id: Id::new(),
            name: "worktree-test".to_string(),
            name_source: RunNameSource::Haiku,
            status: RunStatus::Pending,
            workspace_root: "/workspace".to_string(),
            spec_path: "/workspace/spec.md".to_string(),
            plan_path: Some("/workspace/plan.md".to_string()),
            worktree: Some(RunWorktree {
                base_branch: "main".to_string(),
                run_branch: "run/worktree-test".to_string(),
                merge_target_branch: Some("agent/feature".to_string()),
                merge_strategy: MergeStrategy::Squash,
                worktree_path: "../repo.run-worktree-test".to_string(),
                provider: WorktreeProvider::default(),
            }),
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: Some(r#"{"model":"opus"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };

        ts.storage.insert_run(&run).await.unwrap();
        let retrieved = ts.storage.get_run(&run.id).await.unwrap();

        // Verify worktree fields.
        let wt = retrieved.worktree.unwrap();
        assert_eq!(wt.base_branch, "main");
        assert_eq!(wt.run_branch, "run/worktree-test");
        assert_eq!(wt.merge_target_branch, Some("agent/feature".to_string()));
        assert_eq!(wt.merge_strategy, MergeStrategy::Squash);
        assert_eq!(wt.worktree_path, "../repo.run-worktree-test");
        assert_eq!(wt.provider, WorktreeProvider::Auto);

        // Verify config.
        assert_eq!(
            retrieved.config_json,
            Some(r#"{"model":"opus"}"#.to_string())
        );
    }

    #[tokio::test]
    async fn worktree_provider_round_trip() {
        let ts = create_test_storage().await;
        let now = Utc::now();

        // Test each provider variant round-trips correctly.
        for (provider, expected_str) in [
            (WorktreeProvider::Auto, "auto"),
            (WorktreeProvider::Worktrunk, "worktrunk"),
            (WorktreeProvider::Git, "git"),
        ] {
            let run = Run {
                id: Id::new(),
                name: format!("provider-test-{}", expected_str),
                name_source: RunNameSource::Haiku,
                status: RunStatus::Pending,
                workspace_root: "/workspace".to_string(),
                spec_path: "/workspace/spec.md".to_string(),
                plan_path: None,
                worktree: Some(RunWorktree {
                    base_branch: "main".to_string(),
                    run_branch: format!("run/test-{}", expected_str),
                    merge_target_branch: None,
                    merge_strategy: MergeStrategy::Squash,
                    worktree_path: format!("../repo.{}", expected_str),
                    provider,
                }),
                worktree_cleanup_status: None,
                worktree_cleaned_at: None,
                config_json: None,
                created_at: now,
                updated_at: now,
            };

            ts.storage.insert_run(&run).await.unwrap();
            let retrieved = ts.storage.get_run(&run.id).await.unwrap();

            let wt = retrieved.worktree.unwrap();
            assert_eq!(
                wt.provider, provider,
                "Provider {} did not round-trip",
                expected_str
            );
        }
    }

    #[tokio::test]
    async fn update_step_updates_fields() {
        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        let step = Step {
            id: Id::new(),
            run_id: run.id.clone(),
            phase: StepPhase::Implementation,
            status: StepStatus::InProgress,
            attempt: 1,
            started_at: Some(Utc::now()),
            ended_at: None,
            exit_code: None,
            prompt_path: Some("/workspace/prompt.txt".to_string()),
            output_path: None,
        };
        ts.storage.insert_step(&step).await.unwrap();

        // Update step with completion data.
        ts.storage
            .update_step(
                &step.id,
                StepStatus::Succeeded,
                Some(0),
                Some("/workspace/output.log"),
            )
            .await
            .unwrap();

        let updated = ts.storage.get_step(&step.id).await.unwrap();
        assert_eq!(updated.status, StepStatus::Succeeded);
        assert_eq!(updated.exit_code, Some(0));
        assert_eq!(
            updated.output_path,
            Some("/workspace/output.log".to_string())
        );
        assert!(updated.ended_at.is_some());
    }

    #[tokio::test]
    async fn event_with_step_id_persists() {
        use loop_core::events::StepStartedPayload;

        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        let step = Step {
            id: Id::new(),
            run_id: run.id.clone(),
            phase: StepPhase::Review,
            status: StepStatus::Queued,
            attempt: 2,
            started_at: None,
            ended_at: None,
            exit_code: None,
            prompt_path: None,
            output_path: None,
        };
        ts.storage.insert_step(&step).await.unwrap();

        let payload = EventPayload::StepStarted(StepStartedPayload {
            step_id: step.id.clone(),
            phase: "review".to_string(),
            attempt: 2,
        });
        ts.storage
            .append_event(&run.id, Some(&step.id), &payload)
            .await
            .unwrap();

        let events = ts.storage.list_events(&run.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].step_id, Some(step.id));
        assert_eq!(events[0].event_type, "STEP_STARTED");
    }

    #[tokio::test]
    async fn multiple_events_preserve_order() {
        use loop_core::events::{RunCreatedPayload, RunStartedPayload};

        let ts = create_test_storage().await;
        let run = create_test_run();
        ts.storage.insert_run(&run).await.unwrap();

        // Append events in sequence.
        let payload1 = EventPayload::RunCreated(RunCreatedPayload {
            run_id: run.id.clone(),
            name: run.name.clone(),
            name_source: run.name_source,
            spec_path: run.spec_path.clone(),
            plan_path: run.plan_path.clone(),
        });
        ts.storage
            .append_event(&run.id, None, &payload1)
            .await
            .unwrap();

        // Small delay to ensure different timestamps.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let payload2 = EventPayload::RunStarted(RunStartedPayload {
            run_id: run.id.clone(),
            worker_id: "worker-1".to_string(),
        });
        ts.storage
            .append_event(&run.id, None, &payload2)
            .await
            .unwrap();

        let events = ts.storage.list_events(&run.id).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "RUN_CREATED");
        assert_eq!(events[1].event_type, "RUN_STARTED");
        assert!(events[0].timestamp <= events[1].timestamp);
    }

    #[tokio::test]
    async fn count_running_runs_for_workspace() {
        let ts = create_test_storage().await;
        let now = Utc::now();

        // Create runs in different workspaces and statuses.
        let run1 = Run {
            id: Id::new(),
            name: "run1".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Running,
            workspace_root: "/workspace-a".to_string(),
            spec_path: "/workspace-a/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let run2 = Run {
            id: Id::new(),
            name: "run2".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Running,
            workspace_root: "/workspace-a".to_string(),
            spec_path: "/workspace-a/spec2.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let run3 = Run {
            id: Id::new(),
            name: "run3".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Pending,
            workspace_root: "/workspace-a".to_string(),
            spec_path: "/workspace-a/spec3.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let run4 = Run {
            id: Id::new(),
            name: "run4".to_string(),
            name_source: RunNameSource::SpecSlug,
            status: RunStatus::Running,
            workspace_root: "/workspace-b".to_string(),
            spec_path: "/workspace-b/spec.md".to_string(),
            plan_path: None,
            worktree: None,
            worktree_cleanup_status: None,
            worktree_cleaned_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };

        ts.storage.insert_run(&run1).await.unwrap();
        ts.storage.insert_run(&run2).await.unwrap();
        ts.storage.insert_run(&run3).await.unwrap();
        ts.storage.insert_run(&run4).await.unwrap();

        // workspace-a has 2 RUNNING runs (run1, run2), not counting PENDING run3.
        let count_a = ts
            .storage
            .count_running_runs_for_workspace("/workspace-a")
            .await
            .unwrap();
        assert_eq!(count_a, 2);

        // workspace-b has 1 RUNNING run.
        let count_b = ts
            .storage
            .count_running_runs_for_workspace("/workspace-b")
            .await
            .unwrap();
        assert_eq!(count_b, 1);

        // workspace-c has 0 runs.
        let count_c = ts
            .storage
            .count_running_runs_for_workspace("/workspace-c")
            .await
            .unwrap();
        assert_eq!(count_c, 0);
    }
}
