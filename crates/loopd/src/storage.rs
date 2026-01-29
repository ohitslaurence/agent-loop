//! SQLite storage module for the orchestrator daemon.
//!
//! Implements persistence for runs, steps, events, and artifacts.
//! See spec Section 3.2 and Section 4.2.

use chrono::{DateTime, Utc};
use loop_core::{
    events::EventPayload, Artifact, ArtifactLocation, Event, Id, MergeStrategy, Run, RunNameSource,
    RunStatus, RunWorktree, Step, StepPhase, StepStatus,
};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
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
        // Read and execute the init migration
        let init_sql = include_str!("../../../migrations/0001_init.sql");

        // Remove comment lines before splitting
        let cleaned: String = init_sql
            .lines()
            .filter(|line| !line.trim().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");

        for statement in cleaned.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    // --- Run operations ---

    /// Insert a new run.
    pub async fn insert_run(&self, run: &Run) -> Result<()> {
        let name_source = run.name_source.as_str();
        let status = run.status.as_str();
        let (base_branch, run_branch, merge_target, merge_strategy, worktree_path) =
            match &run.worktree {
                Some(wt) => (
                    Some(wt.base_branch.as_str()),
                    Some(wt.run_branch.as_str()),
                    wt.merge_target_branch.as_deref(),
                    Some(wt.merge_strategy.as_str()),
                    Some(wt.worktree_path.as_str()),
                ),
                None => (None, None, None, None, None),
            };
        let created_at = run.created_at.timestamp_millis();
        let updated_at = run.updated_at.timestamp_millis();

        sqlx::query(
            r#"
            INSERT INTO runs (id, name, name_source, status, workspace_root, spec_path, plan_path,
                              base_branch, run_branch, merge_target_branch, merge_strategy,
                              worktree_path, config_json, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
        .bind(&run.config_json)
        .bind(created_at)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a run by ID.
    pub async fn get_run(&self, id: &Id) -> Result<Run> {
        let row = sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = ?1")
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
                sqlx::query_as::<_, RunRow>(
                    "SELECT * FROM runs WHERE workspace_root = ?1 ORDER BY created_at DESC",
                )
                .bind(ws)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as::<_, RunRow>("SELECT * FROM runs ORDER BY created_at DESC")
                    .fetch_all(&self.pool)
                    .await?
            }
        };

        Ok(rows.into_iter().map(|r| r.into_run()).collect())
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
}
