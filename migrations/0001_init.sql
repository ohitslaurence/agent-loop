-- Initial schema for loopd orchestrator daemon
-- See spec: specs/orchestrator-daemon.md Section 3.2

-- Enable WAL mode for better concurrency
PRAGMA journal_mode = WAL;

-- Runs table: source of truth for run lifecycle
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    name_source TEXT NOT NULL CHECK (name_source IN ('spec_slug', 'haiku')),
    status TEXT NOT NULL CHECK (status IN ('PENDING', 'RUNNING', 'PAUSED', 'COMPLETED', 'FAILED', 'CANCELED')),
    workspace_root TEXT NOT NULL,
    spec_path TEXT NOT NULL,
    plan_path TEXT,
    -- Worktree fields (flattened from RunWorktree)
    base_branch TEXT,
    run_branch TEXT,
    merge_target_branch TEXT,
    merge_strategy TEXT CHECK (merge_strategy IS NULL OR merge_strategy IN ('none', 'merge', 'squash')),
    worktree_path TEXT,
    -- Config
    config_json TEXT,
    -- Timestamps (Unix epoch milliseconds)
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
CREATE INDEX IF NOT EXISTS idx_runs_workspace ON runs(workspace_root);
CREATE INDEX IF NOT EXISTS idx_runs_created ON runs(created_at);

-- Steps table: tracks iteration attempts
CREATE TABLE IF NOT EXISTS steps (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    phase TEXT NOT NULL CHECK (phase IN ('implementation', 'review', 'verification', 'watchdog', 'merge')),
    status TEXT NOT NULL CHECK (status IN ('QUEUED', 'IN_PROGRESS', 'SUCCEEDED', 'FAILED', 'RETRYING', 'CANCELED')),
    attempt INTEGER NOT NULL DEFAULT 1,
    -- Timestamps (Unix epoch milliseconds)
    started_at INTEGER,
    ended_at INTEGER,
    exit_code INTEGER,
    prompt_path TEXT,
    output_path TEXT
);

CREATE INDEX IF NOT EXISTS idx_steps_run ON steps(run_id);
CREATE INDEX IF NOT EXISTS idx_steps_status ON steps(status);
CREATE INDEX IF NOT EXISTS idx_steps_phase ON steps(phase);

-- Events table: append-only audit log
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    step_id TEXT REFERENCES steps(id) ON DELETE SET NULL,
    type TEXT NOT NULL,
    -- Timestamp (Unix epoch milliseconds)
    ts INTEGER NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_run ON events(run_id);
CREATE INDEX IF NOT EXISTS idx_events_type ON events(type);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);

-- Artifacts table: references to saved files
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    location TEXT NOT NULL CHECK (location IN ('workspace', 'global')),
    path TEXT NOT NULL,
    checksum TEXT
);

CREATE INDEX IF NOT EXISTS idx_artifacts_run ON artifacts(run_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_kind ON artifacts(kind);
