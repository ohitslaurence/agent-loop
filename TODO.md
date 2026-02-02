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

### Terminology confusion
- **"Attempt N" is misleading**: Users think it means retry-after-failure. Actually means "Iteration N" of the impl→review→verify loop. Rename to "Iteration N" or "Loop N".
- **"Review passed" misleading**: Sounds like human review, but it's AI self-review. Rename to "Self-review" or "Code review (automated)".

### Progress indicators
- **No in-progress state in lifecycle**: Only shows ✓ completed or ○ pending. Need animated state for currently running step.
- **In-progress indicators too subtle**: Need better animated spinners/pulsing, not just blue dot.
- **No live elapsed time**: Show "Running for 2m 30s" with live counter for active steps.

### Layout/clarity
- **Details + Worktree stacked**: Should be side-by-side on desktop.
- **Output log phases unclear**: Can't tell which output is from impl vs review vs verify. Add phase headers.
- **Exit code only shown on failure**: Show "Exit 0" for success too - it's meaningful.
- **No step completion ticks**: Steps like Review/Verification show no visual ✓ indicator.

### Missing features
- **No commits view**: CommitList exists but only in review page. Show commits on main run detail.
- **No step output preview**: Show last few lines inline with each step.

## P2 - Run Analytics & Timing

> Track detailed timing/performance data for runs.

### Per-step timing
- API wait time vs execution time
- Token usage (input/output)
- Model used

### Run aggregates
- Total wall-clock time
- Total API time (sum of step durations)
- Iteration count + avg iteration time
- Cost estimate (model pricing)

### Historical trends (future)
- Avg duration by spec/workspace
- Failure rate by phase
- Most expensive runs
