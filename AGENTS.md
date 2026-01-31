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

## Development

Rebuild and install loopd/loopctl after making changes:

```bash
./dev/reinstall
```

This kills any running daemon, builds release binaries, and installs to `~/.local/bin`. Override install location with `INSTALL_DIR=/usr/local/bin ./dev/reinstall`.

Bump the workspace version in `Cargo.toml` when making releases.

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
