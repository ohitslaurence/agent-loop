# Orchestrator Daemon Extended Implementation Plan

Reference: [orchestrator-daemon-extended.md](../orchestrator-daemon-extended.md)

## Phase 1: Runner Pipeline
- [R] Wire `process_run()` to execute runner/verifier/watchdog (See Section 4.2, Section 5.1)
- [R] Persist artifacts and events for each phase (See Section 4.3, Section 7.1)
- [R] Implement completion detection and merge flow (See Section 5.1)

## Phase 2: Local Scaling
- [R] Add `max_runs_per_workspace` enforcement in scheduler (See Section 4.2, Section 6.2)
- [R] Add `queue_policy` handling (fifo vs newest_first) (See Section 5.3)

## Phase 3: CLI Readiness
- [R] Add readiness probe with backoff in `loopctl` before issuing commands (See Section 5.3, Section 7.1)
- [ ] Improve error messages when daemon is unreachable (See Section 6.1)

## Phase 4: Tests and Diagnostics
- [ ] Add tests for runner pipeline progression (See Section 5.1)
- [ ] Add tests for per-workspace caps and queue policy (See Section 6.2)
- [ ] Add tests for readiness backoff behavior (See Section 5.3)

## Files to Modify
- `crates/loopd/src/lib.rs`
- `crates/loopd/src/scheduler.rs`
- `crates/loopd/src/runner.rs`
- `crates/loopd/src/verifier.rs`
- `crates/loopd/src/watchdog.rs`
- `crates/loopd/src/server.rs`
- `crates/loopctl/src/client.rs`
- `crates/loopctl/src/main.rs`

## Verification Checklist
### Implementation Checklist
- [ ] `cargo fmt --check`
- [ ] `cargo test -p loop-core`
- [ ] `cargo test -p loopd`
- [ ] `cargo test -p loopctl`

### Manual QA Checklist (do not mark, human verification)
- [ ]? Run a spec to completion (no stub pause)
- [ ]? Queue two runs in same repo and confirm per-workspace cap
- [ ]? Start `loopd` and run `loopctl` immediately to verify readiness probe

## Notes (Optional)
- Keep `specs/orchestrator-daemon.md` stable; changes live here.

## Learnings
- Wire process_run: Run artifact dir must use `run_id` not `run_name` (spec §3.2 is explicit: "directories remain `run-<run_id>` for compatibility").
- Readiness probe: Section 7.1 requires logging retries - don't forget observability requirements.
