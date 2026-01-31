# TODO

Remaining hardening items from process/concurrency audit (2026-01-31).

## P0 - Edge Case Hardening

### Permit leak on storage failure
`scheduler.rs` - `release_run()` decrements `active_runs` counter but only releases semaphore permit if `update_run_status()` succeeds. If storage fails, semaphore and counter get out of sync.

**Fix:** Replace `mem::forget(permit)` pattern with RAII guard struct that releases on drop:
```rust
struct ConcurrencyGuard {
    semaphore: Arc<Semaphore>,
    active_runs: Arc<AtomicUsize>,
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        self.semaphore.add_permits(1);
        self.active_runs.fetch_sub(1, Ordering::SeqCst);
    }
}
```

### No DB transactions for atomic state changes
`lib.rs` - Run completion writes event, then updates status in separate SQL statements. Partial failure leaves inconsistent state.

**Fix:** Add transaction wrapper to `Storage`:
```rust
pub async fn complete_run_atomically(
    &self,
    run_id: &Id,
    event: &EventPayload,
    status: RunStatus,
) -> Result<()> {
    let mut tx = self.pool.begin().await?;
    // ... both ops in transaction
    tx.commit().await?;
    Ok(())
}
```

## P1 - Loop Resilience

### No consecutive failure detection
`lib.rs` - Verification can fail repeatedly until iteration limit. No detection of "stuck in same failure" pattern.

**Fix:** Track consecutive failures per phase, abort run if threshold exceeded:
```rust
if consecutive_verification_failures >= 3 {
    return Err(RunError::MaxConsecutiveFailures("verification"));
}
```

## P2 - Observability & API

### Metrics endpoint
No Prometheus-style metrics export. Can't observe queue depth, failure rates, latencies without log parsing.

### /runs pagination
Returns all runs; could be thousands. Add `?limit=100&offset=0&status=FAILED`.

### Integration tests for process_run
Only HTTP endpoint tests exist. Need tests for full run lifecycle, watchdog signals, verification requeue.
