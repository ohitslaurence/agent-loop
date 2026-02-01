# TODO

Remaining hardening items from process/concurrency audit (2026-01-31).
See `ROADMAP.md` for longer-term vision (dashboard, agent specialization, etc.).

## P0 - Edge Case Hardening

### ~~Permit leak on storage failure~~ (DONE)
Fixed in `scheduler.rs` - `release_run()` and `cancel_run()` now release permits BEFORE storage calls.
If storage fails, permit is already released (no leak).

### ~~No DB transactions for atomic state changes~~ (DONE)
Added `Storage::complete_run_atomically()` and `Scheduler::complete_run()` - wraps event append + status update
in a single transaction. All run completion/failure paths in `lib.rs` now use this atomic method.

## P1 - Loop Resilience

> Foundation for `ROADMAP.md` Phase 3 (Stuck Detection, circular diff detection, escalation).

### No consecutive failure detection
`lib.rs` - Verification can fail repeatedly until iteration limit. No detection of "stuck in same failure" pattern.

**Fix:** Track consecutive failures per phase, abort run if threshold exceeded:
```rust
if consecutive_verification_failures >= 3 {
    return Err(RunError::MaxConsecutiveFailures("verification"));
}
```

## P2 - Observability & API

> Foundation for `ROADMAP.md` Phase 1 & 4 (Dashboard, notifications, cost tracking).

### Metrics endpoint
No Prometheus-style metrics export. Can't observe queue depth, failure rates, latencies without log parsing.

### /runs pagination
Returns all runs; could be thousands. Add `?limit=100&offset=0&status=FAILED`.

### Integration tests for process_run
Only HTTP endpoint tests exist. Need tests for full run lifecycle, watchdog signals, verification requeue.

## P1 - Dashboard UX Improvements

> Feedback from live usage 2026-02-01.

### Steps section confusion
- **"Attempt 2" unclear**: Users don't know if this means retry (failure), run 2, or loop iteration 2. Need clearer labeling: "Retry 2 (after failure)" vs "Iteration 2".
- **In-progress indicators too subtle**: Need animated spinners/pulsing for running steps, not just blue dot.
- **Exit code only on failure**: Show exit code for all completed steps, not just failures. "Exit 0" is meaningful.
- **No tick marks on completed steps**: Steps like Review/Verification show no visual completion indicator.

### Lifecycle section confusion
- **"Review passed" misleading**: Sounds like human review, but it's the AI review phase. Rename to "Self-review completed" or "Code review (automated)".
- **No in-progress state**: Only shows completed (✓) or pending (○). Need third state for currently running.
- **Steps unclear**: "Implementation, then review passed" - what does this flow mean? Add brief descriptions.

### Layout issues
- **Details and Worktree stacked**: Should be side-by-side on desktop to save vertical space.
- **Output log unclear**: Can't tell which output is from Implementation vs Review vs Verification. Add phase headers/separators.

### Missing features
- **No commits view**: Can't see commits as they come in. Add a CommitList component showing commits on run branch.
- **No live elapsed time**: For in-progress steps, show "Running for 2m 30s" with live counter.
- **No step output preview**: Have to scroll to log viewer - show last few lines inline with each step.
