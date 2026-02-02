# Consecutive Failure Detection

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-02-02

---

## 1. Overview
### Purpose
Stop runs that are stuck in repeated phase failures (especially verification) before the iteration limit, while preserving current loop semantics.

### Goals
- Track consecutive failures per phase using existing step history from `crates/loopd/src/storage.rs`.
- Abort the run when a configured threshold is reached, emitting `RUN_FAILED` with a clear reason.
- Keep iteration limit behavior and watchdog flow unchanged in `crates/loopd/src/lib.rs`.

### Non-Goals
- Changing watchdog signal detection in `crates/loopd/src/watchdog.rs`.
- Adding new database tables or step fields.
- Automatically diagnosing failure causes.

---

## 2. Architecture
### Components
- Run loop and step execution: `process_run()` in `crates/loopd/src/lib.rs`.
- Phase selection: `Scheduler::determine_next_phase()` in `crates/loopd/src/scheduler.rs`.
- Step history: `Storage::list_steps()` in `crates/loopd/src/storage.rs`.
- Configuration: `Config` in `crates/loop-core/src/config.rs`.
- Failure events: `EventPayload::RunFailed` in `crates/loop-core/src/events.rs`.

### Dependencies
- No new external dependencies.

### Module/Folder Layout
- `crates/loopd/src/lib.rs` (consecutive failure tracking + run abort)
- `crates/loop-core/src/config.rs` (new config keys + defaults)
- `crates/loopd/tests/` or `crates/loopd/src/` (tests for consecutive failure abort)

---

## 3. Data Model
### Core Types
- `StepPhase` and `StepStatus` from `crates/loop-core/src/types.rs`.
- `Step` from `crates/loop-core/src/types.rs` for status/phase inspection.

### Configuration
| Key | Type | Default | Description |
| --- | ---- | ------- | ----------- |
| `max_consecutive_verification_failures` | u32 | 3 | Fail the run after N consecutive verification failures. |
| `max_consecutive_review_failures` | u32 | 0 | Fail the run after N consecutive review failures; `0` disables. |

Example `.loop/config`:
```
max_consecutive_verification_failures=3
max_consecutive_review_failures=0
```

### Derived State
`ConsecutiveFailures` is computed from step history at run start and updated per completed step:
- Map `StepPhase -> u32` for consecutive failures.
- A failure increments the phase counter; a success resets it to 0.
- Counters are recomputed from persisted steps via `Storage::list_steps()` to handle restarts.

---

## 4. Interfaces
### Public APIs
- No new endpoints or payloads.

### Internal APIs
- `process_run()` initializes counters from `Storage::list_steps()`.
- After each step completion, update counters for the step phase and evaluate thresholds.
- On threshold breach, call `finalize_run_artifacts()` and `maybe_run_postmortem()`, then `Scheduler::complete_run()`.

### Events (names + payloads)
- Reuse `RUN_FAILED` with a structured reason string.
- Reason format: `max_consecutive_failures:<phase>:<limit>`.

Example payload (from `crates/loop-core/src/events.rs`):
```
EventPayload::RunFailed(RunFailedPayload {
    run_id,
    reason: "max_consecutive_failures:verification:3".to_string(),
})
```

---

## 5. Workflows
### Main Flow
```
verification step failed
  -> increment verification failure counter
  -> if counter < limit: continue (scheduler requeues implementation)
  -> if counter >= limit:
       finalize_run_artifacts()
       maybe_run_postmortem()
       emit RUN_FAILED + mark run FAILED
       stop loop
```

### Edge Cases
- If `max_consecutive_*_failures` is `0`, skip the threshold check for that phase.
- If verification is not configured (no commands), verification passes and resets the counter.
- If the run is canceled or no longer running, exit before evaluating thresholds.
- On daemon restart, counters are rebuilt from stored step history.

### Retry/Backoff
- No new retry logic; uses existing iteration loop and verification requeue behavior.

---

## 6. Error Handling
### Error Types
- Threshold breach triggers `RUN_FAILED` with reason `max_consecutive_failures:<phase>:<limit>`.
- Summary uses `ExitReason::Failed` in `crates/loopd/src/postmortem.rs`.

### Recovery Strategy
- No recovery beyond existing loop requeue; threshold breach ends the run.

---

## 7. Observability
### Logs
- `warn!` when a threshold is reached, include `run_id`, `phase`, `count`, `limit`.
- Optional `info!` when counters reset after a successful step.

### Metrics
- None added in this spec.

### Traces
- None added in this spec.

---

## 8. Security and Privacy
### AuthZ/AuthN
- No changes.

### Data Handling
- No new data persisted beyond existing step records.

---

## 9. Migration or Rollout
### Compatibility Notes
- No schema changes. New config keys are optional and defaulted.

### Rollout Plan
1. Add config defaults and parsing.
2. Add counter initialization and threshold evaluation in `process_run()`.
3. Add tests for consecutive verification failure abort.

---

## 10. Open Questions
- Should review failures be enabled by default (non-zero threshold), or remain disabled?
