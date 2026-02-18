# Agent Loop Orchestrator Guidelines

## Vision
Build a reliable, extensible orchestration daemon that runs agent loops across multiple workspaces with strong observability, safe defaults, and a clear path to UI/streaming control.

## Core Principles
- Reliability first: crash recovery, durable state, resumable runs.
- Clarity over cleverness: readable, modular, idiomatic Rust.
- Extensibility: new phases/transports/policies without rewrites.
- Debuggability: rich events, inspectable state, streaming output.
- Compatibility: preserve current loop semantics and artifact layout.

## Design Guardrails
- Persist metadata in SQLite; store heavy artifacts on disk.
- Keep deterministic run directories by run_id; expose human-readable names in UI/CLI.
- Workspace-scoped runs; per-repo configs must continue to work.
- HTTP + SSE for control and streaming; localhost-only with auth token.

## Quality Bar
- Small, reviewable diffs.
- Tests for core logic, storage, runner, and HTTP/SSE behavior.
- No hidden dependencies; document all defaults and fallbacks.

## Code Patterns

### Process Handling
- **Always kill before reap**: On timeout/cancellation, call `child.kill().await` then `child.wait().await` to prevent zombies.
- **Use real time for timeouts**: Track elapsed time with `Instant::now()`, not heartbeat counters. Heartbeat-based timeouts can overshoot by one interval.
- **Timeout I/O capture**: Wrap stdout/stderr task joins with `tokio::time::timeout()` to prevent hangs on stuck pipes.
- **Handle SIGTERM**: Containers send SIGTERM, not SIGINT. Handle both for graceful shutdown.

### Concurrency
- **Use `Arc::clone(&x)`** not `x.clone()` for ref-counted types—makes intent explicit.
- **Guard structs over `mem::forget`**: Don't leak semaphore permits; use RAII guards that release on drop.
- **Lock ordering**: If multiple locks exist, document acquisition order to prevent deadlocks.

### Resource Management
- **Bound output buffers**: External processes can produce unbounded output. Cap at a reasonable limit (e.g., 100MB) to prevent OOM.
- **Scale pools with concurrency**: Connection pools, thread pools, and semaphores should scale with `max_concurrent_runs`, not be hardcoded.
- **Check disk space**: Before writing artifacts, verify sufficient space exists.

### Database
- **Atomic state changes**: Run status + event appends that must be consistent should use transactions.
- **Validate state before transition**: Check current status before updating to catch races.
- **Idempotent operations**: API endpoints that modify state should handle retries gracefully.

### Error Handling
- **`let _ =` with comment**: When intentionally ignoring errors, use `let _ = ...` and add a comment explaining why.
- **Preserve error context**: Avoid `map_err(|_| ...)` unless the replacement error includes the relevant context (key, value, operation).

### Anti-Patterns to Avoid
- ❌ `timeout(child.wait_with_output())` without kill—leaves zombie on timeout
- ❌ Unbounded `read_to_end()` on untrusted process output
- ❌ Counting heartbeats instead of checking `Instant::elapsed()`
- ❌ `.ok()` to silently discard errors (use `let _ =` with comment instead)
- ❌ Separate SQL statements for operations that must be atomic
- ❌ Fixed pool sizes that don't account for configured concurrency

## Workflow
- Specs live in `specs/` and plans in `specs/planning/`.
- Update `specs/README.md` when adding a spec or plan.
- Follow the current spec template and cite sections in plan tasks.

## Data Locations

- **Daemon binaries:** `~/.local/bin/loopd`, `~/.local/bin/loopctl`
- **Database:** `~/.local/share/loopd/loopd.db` (SQLite)
- **Global run artifacts:** `~/.local/share/loopd/runs/run-<uuid>/`
- **Workspace-local run artifacts:** `<workspace_root>/logs/loop/run-<uuid>/`

Both global and workspace-local directories contain run artifacts. The workspace-local copy may have files the global copy doesn't (e.g. `runner-notes.txt`, `review-prompt.txt`, analysis subdirectory). Always check both when debugging.

Run artifact files:
  - `iter-NN-impl.log` / `iter-NN-review.log` — full iteration output
  - `iter-NN-impl.tail.txt` / `iter-NN-review.tail.txt` — last 200 lines
  - `summary.json` — run metadata, exit reason, timing
  - `report.tsv` — timeline of all events
  - `prompt.txt` — the prompt used for the run
  - `review-prompt.txt` — the review prompt (workspace-local only)
  - `runner-notes.txt` — runner notes (workspace-local only)

The global log dir is configured via `global_log_dir` (default: `dirs::data_local_dir()/loopd`, typically `~/.local/share/loopd`). The workspace-local log dir is configured via `log_dir` (default: `logs/loop`).

- **Analysis tools:** `./bin/loop-analyze [run-id]` - generates postmortem prompts

To analyze a run:
```bash
./bin/loop-analyze                    # latest run
./bin/loop-analyze <run-id> --run     # run full postmortem
```

## Development

Rebuild and install loopd/loopctl after making changes:

```bash
./dev/reinstall
```

This kills any running daemon, builds release binaries, and installs to `~/.local/bin`. Override install location with `INSTALL_DIR=/usr/local/bin ./dev/reinstall`.

Bump the workspace version in `Cargo.toml` when making releases.

## Dashboard/Review Notes

- loopd serializes enums as SCREAMING_SNAKE_CASE or snake_case; dashboard code expects PascalCase. Normalize `RunStatus`, `StepStatus`, `StepPhase`, `ReviewStatus`, `MergeStrategy`, `WorktreeProvider` on the client.
- SSE event names match `loop_core::events::EventType` (SCREAMING_SNAKE_CASE, e.g. `RUN_STARTED`, `STEP_FINISHED`, `POSTMORTEM_START`). Subscribe using those names or normalize before matching.
- `STEP_FINISHED` payload does not include `phase`; use steps data (or step_id→step lookup) for lifecycle status instead of inferring from events alone.
- Review diff UI lives at `/runs/$runId/review` and calls `GET /runs/{id}/diff`. If the link is missing, check run status normalization and that `run.worktree.run_branch` is present.

## Run Lifecycle

A run executes in a loop of **implementation** and **review** phases (steps). Each iteration is one impl+review pair.

- The **reviewer approves or rejects each iteration**, not the entire run. An approval means "this iteration's changes are acceptable," not "the task is complete."
- After reviewer approval, the loop continues with the next implementation step. The run only finishes when: the agent signals completion (exit reason `complete_plan` or `complete_reviewer`), the iteration limit is reached, or a failure occurs.
- `exit_reason` in `summary.json` tells you why a run stopped: `complete_plan`, `complete_reviewer`, `claude_failed`, `iteration_limit`, `cancelled`, etc.
- `claude_failed` with `last_exit_code: 0` typically means an external failure (API 500, network issue), not a code/logic error. Check the last `iter-NN-*.tail.txt` for the actual error message.

### Debugging a failed run

1. Find the run: `ls ~/.local/share/loopd/runs/ | sort -r | head -5`
2. Check `summary.json` for `exit_reason` and `last_exit_code`
3. Read the last iteration's tail file for the actual error
4. Check both global (`~/.local/share/loopd/runs/`) and workspace-local (`<workspace>/logs/loop/`) directories — the workspace copy may have more files
5. The step ID in error logs is a UUIDv7; its timestamp prefix helps correlate with run IDs

## Non-Goals (v0.1)
- Distributed scheduling.
- Web UI.
- Full experiment-mode parity.

## Repo Status (2026-01-29)
- Orchestrator daemon + extended plan work is implemented; runner/verifier/watchdog pipeline is wired in `crates/loopd/src/lib.rs`.
- `loopctl` readiness probe with backoff is implemented in `crates/loopctl/src/client.rs`.
- Local scaling (`max_runs_per_workspace`, queue policy) is implemented in scheduler.
- `ARCHITECTURE.md` reflects the current execution flow; if docs and code disagree, defer to code.
- Worktrunk integration is implemented (`specs/worktrunk-integration.md` + plan). Distributed scheduling is deferred (`specs/distributed-scheduling.md`).
