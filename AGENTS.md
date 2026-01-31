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
