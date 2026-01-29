# Architecture Overview

This doc captures the current system architecture for the Agent Loop Orchestrator and highlights the next steps. Specs live in `specs/` and implementation plans in `specs/planning/`.

## Current Shape
- **Rust workspace**: `loop-core`, `loopd`, `loopctl`.
- **Daemon**: `loopd` owns scheduling, storage, HTTP/SSE control plane, and run execution.
- **CLI**: `loopctl` talks to the daemon over localhost HTTP.
- **Storage**: SQLite for metadata + events; artifacts on disk (mirrored).
- **Worktrees**: git worktree lifecycle and merge utilities exist in `crates/loopd/src/git.rs`.

## Components
- `crates/loop-core/`
  - Config parsing (`config.rs`), prompt assembly (`prompt.rs`), completion detection (`completion.rs`), event types (`events.rs`), artifact helpers (`artifacts.rs`).
- `crates/loopd/`
  - HTTP server (`server.rs`), scheduler (`scheduler.rs`), runner (`runner.rs`), watchdog (`watchdog.rs`), verifier (`verifier.rs`), git/worktree utilities (`git.rs`), naming (`naming.rs`).
- `crates/loopctl/`
  - CLI client + output rendering (`client.rs`, `render.rs`).

## Runtime Flow (Target)
```
loopctl run -> loopd scheduler
  -> worktree create (git)
  -> implementation -> review -> verification
  -> watchdog (if signals) -> retry
  -> completion detection -> merge to target branch
```

## Runtime Flow (Actual Today)
- `loopd` starts and serves HTTP + SSE.
- `loopctl run` creates a run in SQLite.
- Runs execute through implementation/review/verification with watchdog evaluation and completion detection.
- Merge phase runs when configured before marking a run completed.

## Storage and Artifacts
- **SQLite**: runs/steps/events/artifacts (`crates/loopd/src/storage.rs`).
- **Artifacts**: `logs/loop/run-<id>/` in workspace + global mirror at `~/.local/share/loopd/runs/run-<id>/`.
- **Names**: run IDs are UUIDv7; human-readable names default to Claude `haiku`.

## Control Plane
- HTTP: `127.0.0.1:7700` (auth token optional via `LOOPD_AUTH_TOKEN`).
- SSE: `/runs/{id}/events` and `/runs/{id}/output`.

## Worktrees and Merge
- Default run branch: `run/<run_name_slug>`.
- Merge target branch: `agent/<spec_slug>` (squash by default).
- Worktree path template: `../{{ repo }}.{{ run_branch | sanitize }}`.
- Git support is implemented in `crates/loopd/src/git.rs`.

## Observability
- Structured logs via `tracing`.
- `report.tsv` and `summary.json` artifacts for parity with legacy loop tooling.
- `loopctl inspect` and `tail` provide run visibility.

## Tests
- Unit and integration tests across core, daemon, CLI, SSE.
- Runner tests stub external commands; no live `claude` required.

## Known Gaps
- Worktrunk provider integration is in progress (see `specs/worktrunk-integration.md`).
- Distributed scheduling is still spec-only and not implemented.

## Next Steps
- Finish Worktrunk provider integration (see `specs/worktrunk-integration.md`).
- Validate Worktrunk flows with manual QA from the plan.
- Revisit distributed scheduling after Worktrunk is stable.
