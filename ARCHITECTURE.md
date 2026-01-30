# Architecture Overview

This doc captures the current system architecture for the Agent Loop Orchestrator and highlights the next steps. Specs live in `specs/` and implementation plans in `specs/planning/`.

## Current Shape
- **Rust workspace**: `loop-core`, `loopd`, `loopctl`.
- **Daemon**: `loopd` owns scheduling, storage, HTTP/SSE control plane, and run execution.
- **CLI**: `loopctl` talks to the daemon over localhost HTTP.
- **Storage**: SQLite for metadata + events; artifacts on disk (mirrored).
- **Worktrees**: git worktree lifecycle and merge utilities exist in `crates/loopd/src/git.rs`.
- **Prompt customization**: `.loop/prompt.txt` or `prompt_file`/`context_files` in `.loop/config`.

## Components
- `crates/loop-core/`
  - Config parsing (`config.rs`), prompt assembly (`prompt.rs`), completion detection (`completion.rs`), event types (`events.rs`), artifact helpers (`artifacts.rs`).
- `crates/loopd/`
  - HTTP server (`server.rs`), scheduler (`scheduler.rs`), runner (`runner.rs`), watchdog (`watchdog.rs`), verifier (`verifier.rs`), git/worktree utilities (`git.rs`), naming (`naming.rs`), postmortem analysis (`postmortem.rs`).
- `crates/loopctl/`
  - CLI client + output rendering (`client.rs`, `render.rs`), analyze command for on-demand postmortem.

## Runtime Flow (Daemon)
```
loopctl run -> loopd scheduler
  -> load config + resolve worktree provider
  -> build worktree config + create worktree (git/worktrunk)
  -> implementation -> review -> verification
  -> watchdog (if signals) -> retry
  -> completion detection -> optional merge
  -> optional worktree cleanup
```

## Storage and Artifacts
- **SQLite**: runs/steps/events/artifacts (`crates/loopd/src/storage.rs`).
- **Artifacts**: `logs/loop/run-<id>/` in workspace + global mirror at `~/.local/share/loopd/runs/run-<id>/`.
- **Names**: run IDs are UUIDv7; human-readable names default to Claude `haiku`.

## Control Plane
- HTTP: `127.0.0.1:7700` (auth token optional via `LOOPD_AUTH_TOKEN`).
- SSE: `/runs/{id}/events` and `/runs/{id}/output`.

## Worktrees and Merge
- Default run branch prefix: `run/` (branch name is `run/<run_name_slug>`).
- Merge target branch is optional (default: none). Merge strategy defaults to squash but only applies when a target is set.
- Worktree path template: `../{{ repo }}.{{ run_branch | sanitize }}` (overridable).
- Provider selection: `auto` (Worktrunk if `wt` is available, else git), `worktrunk`, or `git`.
- Worktrunk provider uses `wt switch --create <run_branch>` and optional `wt remove` on cleanup.

## Postmortem and Summary
- `summary.json`: Written at run end if `summary_json=true` (default). Contains run metadata, timings, exit reason, and artifact paths.
- Postmortem analysis: If `postmortem=true` (default) and `claude` CLI is available, generates analysis reports after run completion:
  - `analysis/run-quality.md`: End-of-task behavior and improvements.
  - `analysis/spec-compliance.md`: Implementation vs spec comparison.
  - `analysis/summary.md`: Synthesized root cause and changes.
  - Git snapshots: `git-status.txt`, `git-last-commit.txt`, `git-last-commit.patch`, `git-diff.patch`.
- HTTP endpoints: `POST /runs/{id}/postmortem` (trigger analysis), `GET /runs/{id}/postmortem` (list artifacts).
- CLI: `loopctl analyze <run_id>` or `loopctl analyze --latest`.
- Events: `POSTMORTEM_START` and `POSTMORTEM_END` emitted around analysis execution.

## Observability
- Structured logs via `tracing`.
- `report.tsv` plus event history in SQLite.
- `loopctl inspect` and `tail` provide run visibility.

## Tests
- Unit and integration tests across core, daemon, CLI, SSE.
- Runner tests stub external commands; no live `claude` required.

## Known Gaps
- Distributed scheduling is deferred.

## Next Steps
- Experiment mode analysis (metrics selection) is out of scope for v1.
- Consider deprecating `bin/loop-analyze` once daemon postmortem parity is validated.
