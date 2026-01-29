# Distributed Scheduling and Worker Pool Implementation Plan

Reference: [distributed-scheduling.md](../distributed-scheduling.md)

## Phase 1: Storage Abstraction
- [ ] Add storage trait and Postgres backend (See Section 2.1, Section 3.2)
- [ ] Add workers/leases tables and migrate runs schema (See Section 3.2)
- [ ] Add config flags for distributed mode and artifact store (See Section 4.1)

## Phase 2: Controller Service
- [ ] Implement controller binary with HTTP APIs for worker registration/claims (See Section 4.1)
- [ ] Implement lease management and requeue logic (See Section 5.3)
- [ ] Emit worker and lease events (See Section 4.3)

## Phase 3: Worker Service
- [ ] Implement worker binary with heartbeat + claim loop (See Section 5.1)
- [ ] Execute steps using existing runner/verifier modules (See Section 5.1)
- [ ] Stream events and completion to controller (See Section 4.1)

## Phase 4: Workspace and Artifacts
- [ ] Implement workspace resolver (shared_fs + git clone) (See Section 5.2)
- [ ] Add artifact store interface with local + S3 backends (See Section 2.1, Section 6.2)

## Phase 5: Tests and Reliability
- [ ] Add integration tests for lease expiration and requeue (See Section 5.3, Section 6.2)
- [ ] Add worker offline recovery tests (See Section 6.2)
- [ ] Add controller/worker API tests (See Section 4.1)

## Files to Create
- `crates/loopd-controller/src/main.rs`
- `crates/loopd-worker/src/main.rs`
- `crates/loopd-controller/src/lease.rs`
- `crates/loopd-worker/src/heartbeat.rs`
- `crates/loop-core/src/storage.rs`

## Files to Modify
- `crates/loop-core/src/types.rs`
- `crates/loop-core/src/config.rs`
- `crates/loopd/src/storage.rs`
- `crates/loopd/src/server.rs`
- `migrations/0003_distributed_mode.sql`

## Verification Checklist
### Implementation Checklist
- [ ] `cargo fmt --check`
- [ ] `cargo test -p loop-core`
- [ ] `cargo test -p loopd-controller`
- [ ] `cargo test -p loopd-worker`

### Manual QA Checklist (do not mark, human verification)
- [ ]? Register a worker and execute a simple run
- [ ]? Kill a worker mid-step and confirm lease requeue
- [ ]? Validate artifact upload and retrieval

## Notes (Optional)
- Keep single-host mode operational throughout the rollout.
