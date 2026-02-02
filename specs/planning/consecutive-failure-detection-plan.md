# Consecutive Failure Detection Implementation Plan

Reference: [consecutive-failure-detection.md](../consecutive-failure-detection.md)

## Checkbox Legend
- [ ] Pending (blocks completion)
- [~] Blocked (blocks completion)
- [x] Implemented, awaiting review
- [R] Reviewed/verified (non-blocking)
- [ ]? Manual QA only (ignored)

## Phase 1: Configuration + Counter Initialization
- [x] Add config keys, defaults, and parsing for consecutive failure thresholds (See Section 3.2)
- [ ] Initialize per-phase consecutive failure counters from step history (See Section 3.3, Section 5.1)

## Phase 2: Threshold Enforcement
- [ ] Enforce threshold checks after failed review/verification steps and abort runs with `RUN_FAILED` reason (See Section 4.2, Section 5.1, Section 6.1)

## Phase 3: Tests
- [ ] Add tests that consecutive verification failures abort before iteration limit (See Section 5.1, Section 6.1)

## Files to Create
- `crates/loopd/tests/consecutive_failure_detection.rs`

## Files to Modify
- `crates/loop-core/src/config.rs`
- `crates/loopd/src/lib.rs`

## Verification Checklist
### Implementation Checklist
- [R] `cargo fmt --check`
- [R] `cargo test -p loop-core`
- [ ] `cargo test -p loopd`

### Manual QA Checklist (do not mark, human verification)
- [ ]? Run a loop with failing verification and confirm abort after N consecutive failures

## Notes (Optional)
- If review threshold remains disabled (`0`), focus tests on verification failure behavior.
