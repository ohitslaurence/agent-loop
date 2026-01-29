# Orchestrator Daemon Extended

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-01-29

---

## 1. Overview
### Purpose
Extend the existing orchestrator daemon with full run execution, local scaling controls, and CLI readiness behavior. This spec focuses on completing the runner pipeline and runtime polish without changing the core architecture.

### Goals
- Wire the runner/verifier/watchdog pipeline so runs fully execute (no stub pause).
- Add `loopctl` readiness probing with backoff against `/health`.
- Implement local scaling controls: per-workspace caps and queue policy.
- Preserve existing storage schema, artifact layout, and HTTP API.

### Non-Goals
- Distributed scheduling or multi-host workers.
- Web UI or remote access.
- Worktrunk integration (covered in `worktrunk-integration.md`).

---

## 2. Architecture
### Components
- `loopd` (daemon): run processing pipeline + scheduler.
- `loopctl` (CLI): readiness probing + clearer errors.
- `loop-core`: completion detection, prompts, and config.

### Dependencies
- No new external dependencies required.

### Module/Folder Layout
Existing modules; no new crates required.

---

## 3. Data Model
### Core Types
- Use existing `Run`, `Step`, `Event` types; no new fields.

### Storage Schema
- No schema changes required.

---

## 4. Interfaces
### Public APIs
No changes to HTTP endpoints or payload shapes.

### Internal APIs
- `process_run()` must execute phases via `runner`, `verifier`, `watchdog`.
- `scheduler` must enforce per-workspace run caps.

### Events (names + payloads)
- Emit existing `STEP_*`, `RUN_*`, and `WATCHDOG_REWRITE` events at each phase.

---

## 5. Workflows
### Main Flow
```
run claimed -> worktree created -> implementation
  -> review -> verification -> watchdog
  -> completion detection -> merge -> completed
```

### Edge Cases
- Verification failure requeues implementation and writes runner notes.
- Watchdog rewrite requeues same phase with new prompt.
- Runner timeout retries; exhaustion fails the run.

### Retry/Backoff
- Claude retry/backoff as configured.
- `loopctl` readiness probe retries for up to 5s (200ms exponential backoff).

---

## 6. Error Handling
### Error Types
- Runner execution errors.
- Verification command failures.
- Watchdog rewrite failures.
- Queue saturation (per-workspace cap hit).

### Recovery Strategy
- Runner errors follow retry policy; on failure mark run FAILED.
- Verification failures requeue implementation.
- Watchdog failure logs and continues without rewrite.
- Queue saturation leaves run PENDING until capacity frees.

---

## 7. Observability
### Logs
- Add log lines when a run is blocked by per-workspace cap.
- Add log lines when readiness probe is retrying.

### Metrics
- Add counters: `queue_blocked_workspace`, `readiness_retry_count`.

---

## 8. Security and Privacy
No changes.

---

## 9. Migration or Rollout
### Compatibility Notes
- No breaking changes expected.

### Rollout Plan
1. Implement runner pipeline in `process_run()`.
2. Add per-workspace cap + queue policy.
3. Add `loopctl` readiness probe.

---

## 10. Open Questions
- Should per-workspace cap be enforced at claim time or enqueue time?
