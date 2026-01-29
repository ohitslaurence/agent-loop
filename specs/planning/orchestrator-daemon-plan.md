# Agent Loop Orchestrator Daemon Implementation Plan

Reference: [orchestrator-daemon.md](../orchestrator-daemon.md)

## Phase 1: Workspace and Storage
- [R] Create Rust workspace and core crate with config and shared types (See Section 2, Section 3)
- [R] Add SQLite migrations for runs/steps/events/artifacts (See Section 3.2)
- [R] Implement storage module with event append + run/step updates (See Section 3.2, Section 4.2)
- [R] Implement run naming (spec_slug and haiku) with fallback behavior (See Section 3.1, Section 4.1)

## Phase 2: Daemon Scheduler and Runner
- [R] Implement daemon main loop, scheduler, and concurrency limit (See Section 2.1, Section 5.1)
- [R] Implement runner to execute Claude CLI with retries/timeouts and artifacts (See Section 5.3, Section 7.1)
- [R] Implement completion detection to match `bin/loop` behavior (See Section 5.1, Section 9.1)
- [R] Implement worktree creation and branch naming defaults (See Section 3, Section 5)
- [R] Implement merge-to-target behavior with squash/merge strategies (See Section 5, Section 6)

## Phase 3: CLI and Control Plane
- [R] Implement `loopctl` commands and output formatting (See Section 4.1)
- [R] Implement local HTTP control plane and auth token (See Section 4.1, Section 8.1)
- [x] Implement SSE endpoints for events and output streaming (See Section 4.1)
- [ ] Update `bin/loop` wrapper to call `loopctl` for compatibility (See Section 9.2)

## Phase 4: Reviewer, Verification, Watchdog
- [ ] Implement review phase scheduling and completion rules (See Section 5.1)
- [ ] Implement verification execution and failure handling (See Section 5.2, Section 6.2)
- [ ] Implement watchdog signals + prompt rewrite policy with audit events (See Section 5.2, Section 4.3)

## Phase 5: Observability and Docs
- [ ] Match artifact layout and export `report.tsv` for `bin/loop-analyze` (See Section 7.1, Section 9.1)
- [ ] Implement artifact mirroring to `global_log_dir` with references stored in SQLite (See Section 3.2, Section 7.1)
- [ ] Update README and install notes for daemon usage (See Section 9.2)

## Phase 6: Testing and Diagnostics
- [ ] Add unit tests for config parsing, completion detection, and run naming (See Section 1.1, Section 4.1, Section 5.1)
- [ ] Add storage tests for migrations and event persistence (See Section 3.2, Section 6.2)
- [ ] Add runner tests for retries/timeouts and exit handling (See Section 5.3, Section 6.2)
- [ ] Add HTTP/SSE integration tests for run lifecycle and streaming (See Section 4.1, Section 7.1)
- [ ] Validate diagnostics output for `loopctl inspect` and `tail` (See Section 7.2)

## Files to Create
- `Cargo.toml`
- `crates/loop-core/src/config.rs`
- `crates/loop-core/src/types.rs`
- `crates/loop-core/src/prompt.rs`
- `crates/loop-core/src/events.rs`
- `crates/loopd/src/main.rs`
- `crates/loopd/src/scheduler.rs`
- `crates/loopd/src/runner.rs`
- `crates/loopd/src/watchdog.rs`
- `crates/loopd/src/storage.rs`
- `crates/loopctl/src/main.rs`
- `crates/loopctl/src/client.rs`
- `crates/loopctl/src/render.rs`
- `migrations/0001_init.sql`

## Files to Modify
- `bin/loop`
- `README.md`
- `install.sh`

## Verification Checklist
### Implementation Checklist
- [ ] `cargo fmt --check`
- [ ] `cargo test -p loop-core`
- [ ] `cargo test -p loopd`
- [ ] `cargo test -p loopctl`

### Manual QA Checklist (do not mark, human verification)
- [ ]? Start daemon and run a small spec through completion
- [ ]? Pause and resume a run mid-iteration
- [ ]? Verify watchdog rewrites are logged and requeued

## Notes (Optional)
- Phase 2: Keep prompt generation aligned with `bin/loop` default prompt.
