# Agent Loop Orchestrator Daemon

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-01-29

---

## 1. Overview
### Purpose
Replace the current bash loop with a reliable Rust daemon that orchestrates long-running agent loops with reviewers, verification, and a watchdog that can auto-rewrite prompts when progress degrades. Preserve plan-mode behavior and artifact layout from `bin/loop` and `lib/agent-loop-ui.sh`.

### Goals
- Reliability first: crash recovery, resumable runs, consistent state.
- Plan-mode parity with `bin/loop` completion detection and prompt structure.
- Concurrency for 2-5 runs with backpressure and per-run isolation.
- Local scale: multiple concurrent runs across worktrees on a single VPS.
- Watchdog can auto-rewrite prompts and requeue steps with audit trail.
- SQLite persistence with append-friendly events.
- Local HTTP API with SSE streaming plus CLI control (start, pause, resume, inspect).
- Artifact mirroring to both workspace and global storage (configurable).
- High code quality: modular, readable, and idiomatic Rust with clear boundaries.
- Extensible architecture: new phases, transports, and policies without rewrites.
- Debuggability: rich event trail, inspectable state, and streaming output.

### Non-Goals
- Multi-host scheduling or distributed queues.
- Web UI or remote API.
- Full experiment-mode parity in v0.1.
- Replacing the Claude CLI (continue to shell out to `claude`).

---

## 2. Architecture
### Components
- `loopd` daemon: scheduler, state machine, process supervisor, HTTP server.
- `loopctl` CLI: local control plane client for HTTP API.
- `loop-core` library: shared types, config parsing, prompt assembly.
- `storage` module: SQLite + event log.
- `runner` module: executes steps via Claude CLI.
- `watchdog` module: evaluates signals and rewrites prompts.
- Transition wrapper: keep `bin/loop` to call `loopctl` for compatibility.

### Dependencies
- Rust: tokio (async runtime), sqlx (SQLite), clap (CLI), serde (config/events), tracing (logs).
- External tools: `claude` CLI, `gritty` for commits (via prompt rules).
- Optional: `gum` remains for bash wrapper, not required by daemon.

### Module/Folder Layout
```
crates/
  loop-core/
    src/{config.rs,types.rs,prompt.rs,events.rs}
  loopd/
    src/{main.rs,scheduler.rs,runner.rs,watchdog.rs,storage.rs}
  loopctl/
    src/{main.rs,client.rs,render.rs}
migrations/
  0001_init.sql
logs/loop/...
bin/loop  (wrapper to loopctl)
```

---

## 3. Data Model
### Core Types
- Run: id, name, name_source, status, workspace_root, spec_path, plan_path, config, created/updated timestamps.
- RunWorktree: base_branch, run_branch, merge_target_branch, merge_strategy, worktree_path.
- Step: id, run_id, phase, status, attempt, timing, exit code, prompt/output paths.
- Event: id, run_id, step_id (optional), type, timestamp, payload.
- Artifact: id, run_id, kind, location, path, checksum.
- WatchdogDecision: signal, action, rewrite_count, notes.

### Enumerations
- RunStatus: PENDING, RUNNING, PAUSED, COMPLETED, FAILED, CANCELED.
- StepPhase: implementation, review, verification, watchdog, merge.
- StepStatus: QUEUED, IN_PROGRESS, SUCCEEDED, FAILED, RETRYING, CANCELED.
- CompletionMode: exact, trailing (match `bin/loop`).
- WatchdogSignal: repeated_task, verification_failed, no_progress, malformed_complete.
- ArtifactLocation: workspace, global.
- RunNameSource: spec_slug, haiku.
- MergeStrategy: none, merge, squash.
- QueuePolicy: fifo, newest_first.

### Storage Schema
SQLite (WAL enabled). Types use TEXT/INTEGER/JSON.

| Table | Key Columns | Notes |
| --- | --- | --- |
| runs | id TEXT PK, name TEXT, name_source TEXT, status TEXT, workspace_root TEXT, spec_path TEXT, plan_path TEXT, base_branch TEXT, run_branch TEXT, merge_target_branch TEXT, merge_strategy TEXT, worktree_path TEXT, config_json TEXT, created_at INTEGER, updated_at INTEGER | Source of truth for run lifecycle |
| steps | id TEXT PK, run_id TEXT, phase TEXT, status TEXT, attempt INTEGER, started_at INTEGER, ended_at INTEGER, exit_code INTEGER, prompt_path TEXT, output_path TEXT | Tracks iteration attempts |
| events | id TEXT PK, run_id TEXT, step_id TEXT, type TEXT, ts INTEGER, payload_json TEXT | Append-only audit log |
| artifacts | id TEXT PK, run_id TEXT, kind TEXT, location TEXT, path TEXT, checksum TEXT | References to saved files |

### Artifact Naming and Paths
Run directories are deterministic by run_id and mirror the current layout:
- Workspace: `<workspace_root>/logs/loop/run-<run_id>/`
- Global: `~/.local/share/loopd/runs/run-<run_id>/`

Artifact files reuse `bin/loop` naming:
- `prompt.txt`, `summary.json`, `report.tsv`
- `iter-XX.log`, `iter-XX.tail.txt`

Run naming:
- `name` is a human-readable label stored in SQLite and shown in CLI/UI.
- Default `name_source=haiku` using Claude model `haiku` to generate a short label at run creation.
- `name_source=spec_slug` uses the spec filename or title.
- `name` is ASCII, max 64 chars; daemon truncates if needed.
- If haiku generation fails, fall back to `spec_slug` and record `name_source=spec_slug`.
- Directories remain `run-<run_id>` for compatibility; name is not used in folder paths.

Worktree and branch defaults:
- `base_branch`: detected default branch (fallback to `main`).
- `run_branch`: `run/<run_name_slug>`.
- `merge_target_branch`: `agent/<spec_slug>`.
- `merge_strategy`: `squash` (only when merge_target_branch is set).
- `worktree_path`: `../{{ repo }}.{{ run_branch | sanitize }}` (sibling to repo).

Worktree path template variables:
- `{{ repo }}`: repository directory name.
- `{{ run_branch }}`: full run branch name.
- `{{ run_branch | sanitize }}`: filesystem-safe branch (slashes replaced with `-`).

---

## 4. Interfaces
### Public APIs
CLI commands (local only):
- `loopctl run <spec> [plan] [--config path]`
- `loopctl list [--status]`
- `loopctl inspect <run_id>`
- `loopctl pause <run_id>`
- `loopctl resume <run_id>`
- `loopctl cancel <run_id>`
- `loopctl tail <run_id>`

### HTTP API
Base: `http://127.0.0.1:<port>` (localhost only)

REST:
- `POST /runs` {spec_path, plan_path, workspace_root, config_override, name?, name_source?, merge_target_branch?, merge_strategy?}
- `GET /runs?workspace_root=...`
- `GET /runs/{id}`
- `POST /runs/{id}/pause`
- `POST /runs/{id}/resume`
- `POST /runs/{id}/cancel`

Streaming (SSE):
- `GET /runs/{id}/events` (structured event stream)
- `GET /runs/{id}/output` (raw iteration output stream)

### Client Behavior
- `loopctl` retries daemon startup with backoff by probing `/health` before failing.
- Retry window default: 5s total, exponential backoff starting at 200ms.
- On timeout, show a clear error including address and auth token hint.

### CLI Flags and Config
- Config format remains key=value in `.loop/config` (see `bin/loop`).
- Precedence: CLI flags > `--config` file > `.loop/config` > defaults.
- Supported keys: specs_dir, plans_dir, log_dir, model, iterations, completion_mode,
  reviewer, verify_cmds, verify_timeout_sec, claude_timeout_sec, claude_retries,
  claude_retry_backoff_sec, artifact_mode, global_log_dir, run_naming_mode,
  run_naming_model, base_branch, run_branch_prefix, merge_target_branch,
  merge_strategy, worktree_path_template, max_concurrency, max_runs_per_workspace,
  queue_policy.
- Environment variable: `LOOP_CONFIG` mirrors current behavior in `bin/loop`.

CLI options for naming:
- `loopctl run --name "..."` (explicit label, overrides auto naming)
- `loopctl run --name-source spec_slug|haiku`
- `loopctl run --name-model haiku` (when name_source=haiku, default)

Default naming config:
- `run_naming_mode=haiku`
- `run_naming_model=haiku`

CLI options for worktrees and merge:
- `loopctl run --base-branch main`
- `loopctl run --run-branch-prefix run/`
- `loopctl run --merge-target agent/<spec_slug>`
- `loopctl run --merge-strategy merge|squash|none`
- `loopctl run --worktree-path-template "../{{ repo }}.{{ run_branch | sanitize }}"`

CLI options for local scale:
- `loopctl run --max-concurrency 5`
- `loopctl run --max-runs-per-workspace 2`
- `loopctl run --queue-policy fifo|newest_first`

### Internal APIs
- Scheduler: `claim_next_run()`, `enqueue_step(run_id, phase)`.
- Runner: `execute_step(step, prompt) -> StepResult`.
- Watchdog: `evaluate(signals) -> WatchdogDecision`.
- Storage: `append_event`, `update_run`, `update_step`.

### Events (Names + Payloads)
- `RUN_CREATED`: {run_id, name, name_source, spec_path, plan_path}
- `RUN_STARTED`: {run_id, worker_id}
- `STEP_STARTED`: {step_id, phase, attempt}
- `STEP_FINISHED`: {step_id, exit_code, duration_ms, output_path}
- `WATCHDOG_REWRITE`: {step_id, signal, prompt_before, prompt_after}
- `RUN_COMPLETED`: {run_id, mode}
- `RUN_FAILED`: {run_id, reason}

Example payload:
```
{
  "type": "WATCHDOG_REWRITE",
  "run_id": "01J2Z8...",
  "step_id": "01J2Z9...",
  "signal": "no_progress",
  "prompt_before": "logs/loop/run-.../prompt.txt",
  "prompt_after": "logs/loop/run-.../prompt.rewrite.1.txt"
}
```

---

## 5. Workflows
### Main Flow
```
spec + plan -> loopctl run -> loopd scheduler
  -> implementation -> review -> verification
  -> watchdog (if signals) -> requeue or continue
  -> completion detection -> final verification -> completed
```

Completion detection honors `exact` and `trailing` modes from `bin/loop`.

Workspace scoping: `loopctl` resolves the repo root and sends it as
`workspace_root`, so runs read `.loop/config` relative to that workspace.

Local scaling:
- Scheduler enforces `max_concurrency` globally (default 3).
- Optional `max_runs_per_workspace` caps concurrent runs per repo.
- Queue discipline controlled by `queue_policy` (default fifo).
- Run branch names are de-duplicated per workspace (append `-2`, `-3` if needed).

### Runner Execution
1. Build phase prompt using `crates/loop-core/src/prompt.rs` (plan-mode format).
2. Execute Claude CLI via `crates/loopd/src/runner.rs`, capturing stdout/stderr.
3. Persist artifacts to workspace + global mirror using `crates/loop-core/src/artifacts.rs`.
4. Append events (`RUN_STARTED`, `STEP_*`) with `crates/loop-core/src/events.rs`.
5. Detect completion using `crates/loop-core/src/completion.rs`.
6. If completion detected, run verification via `crates/loopd/src/verifier.rs`.
7. If verification fails, write runner notes and requeue implementation.
8. Watchdog evaluates signals via `crates/loopd/src/watchdog.rs`; if rewrite, create new prompt and retry the same phase.

### Worktree + Merge Flow
1. Detect `base_branch` and create `run_branch`.
2. Create worktree at `worktree_path` and run loop in that directory.
3. On completion, if `merge_target_branch` is set:
   - Ensure target branch exists (create from base if missing).
   - Merge or squash from `run_branch` into `merge_target_branch`.
   - Leave `merge_target_branch` checked out in the primary worktree.
4. Do not push or open PR automatically in v0.1.

### Edge Cases
- Verification fails: write runner notes, requeue implementation step, do not advance plan.
- Watchdog rewrites prompt: re-run same phase with incremented attempt.
- Daemon restart: resume runs in RUNNING state from last durable step.

### Retry/Backoff
- Claude CLI failures retry N times with backoff (configurable).
- Watchdog rewrite attempts capped (default 2 per run).

---

## 6. Error Handling
### Error Types
- Storage errors (SQLite unavailable or corruption).
- Runner errors (Claude CLI exit, timeout).
- Config errors (invalid paths, missing spec/plan).
- Watchdog errors (malformed signals or rewrite failure).

### Recovery Strategy
- Storage error: mark run FAILED with reason and persist event.
- Runner error: retry per policy; on exhaustion mark step FAILED and run FAILED.
- Config error: fail run before scheduling.
- Watchdog error: log and continue without rewrite for the current step.
- Merge error (conflicts, dirty tree): mark run FAILED with reason and keep run_branch intact.

---

## 7. Observability
### Logs
- Structured logs via tracing to stdout.
- Per-run logs and iteration output stored under `logs/loop/` relative to
  `workspace_root`, plus a mirrored copy under `global_log_dir` (default
  `~/.local/share/loopd`).
- Prompt snapshots and tail logs preserved for `bin/loop-analyze` parity.
- `report.tsv` columns: timestamp_ms, kind, iteration, duration_ms, exit_code,
  output_bytes, output_lines, output_path, message, tasks_done, tasks_total.

### Diagnostics
- `loopctl inspect <run_id>` shows run state, last step, and artifact paths.
- `loopctl tail <run_id>` streams live output (SSE) with reconnect support.
- Event stream can be replayed for a run to reconstruct timeline.
- All errors include run_id and step_id for correlation.

### Metrics
- run_duration_ms, iteration_duration_ms
- verification_pass_count, verification_fail_count
- watchdog_rewrite_count, claude_retry_count

### Traces
- Not required in v0.1.

---

## 8. Security and Privacy
### AuthZ/AuthN
- Local-only HTTP server bound to 127.0.0.1 with auth token header.

### Data Handling
- Prompts and outputs are stored on disk under `logs/loop/`.
- Do not log environment variables unless explicitly required by config.
- Default artifact policy: `artifact_mode=mirror` (workspace + global).

---

## 9. Migration or Rollout
### Compatibility Notes
- Preserve prompt structure and completion detection from `bin/loop`.
- Maintain artifact layout so `bin/loop-analyze` keeps working.
- Continue to read `.loop/config` for per-project settings.
- Allow `log_dir` override to support global logging later.

### Rollout Plan
1. Introduce Rust workspace + daemon + CLI.
2. Add bash wrapper updates in `bin/loop` to call `loopctl`.
3. Keep old path as fallback until v0.1 stabilizes.

---

## 10. Open Questions
- Prompt templating: code-generated vs file-based templates.
- Versioning for watchdog policy rewrites.
- Long-term default log location (workspace vs global).
