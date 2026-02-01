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
