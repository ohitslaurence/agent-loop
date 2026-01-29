# Worktrunk Worktree Integration Implementation Plan

Reference: [worktrunk-integration.md](../worktrunk-integration.md)

## Phase 1: Data Model and Config
- [R] Add WorktreeProvider enum and provider field on RunWorktree (See Section 3)
- [R] Add config keys for Worktrunk provider and CLI flags (See Section 4.1)
- [R] Persist worktree_provider in runs table and storage round-trip (See Section 3.2)

## Phase 2: Provider Implementations
- [R] Add provider interface and git provider adapter around `crates/loopd/src/git.rs` (See Section 2.1, Section 4.2)
- [R] Implement Worktrunk provider using `wt switch --create` and optional `wt step copy-ignored` (See Section 5.3)
- [R] Implement Worktrunk config parsing to resolve worktree-path template (See Section 5.3)

## Phase 3: Runtime Wiring
- [x] Resolve provider per run and emit WORKTREE_PROVIDER_SELECTED events (See Section 5.2)
- [ ] Use provider to create worktree before execution and emit WORKTREE_CREATED (See Section 5.1)
- [ ] Implement optional cleanup and emit WORKTREE_REMOVED (See Section 5.4)

## Phase 4: Tests and Observability
- [ ] Add unit tests for provider selection and config parsing (See Section 5.2)
- [ ] Add integration tests for Worktrunk missing -> fallback/error (See Section 6.2)
- [ ] Verify event payloads serialize as expected (See Section 4.3)

## Files to Create
- `crates/loopd/src/worktree.rs`
- `crates/loopd/src/worktree_worktrunk.rs`

## Files to Modify
- `crates/loop-core/src/config.rs`
- `crates/loop-core/src/types.rs`
- `crates/loop-core/src/events.rs`
- `crates/loopd/src/server.rs`
- `crates/loopd/src/scheduler.rs`
- `crates/loopd/src/runner.rs`
- `crates/loopd/src/storage.rs`
- `crates/loopd/src/git.rs`
- `crates/loopctl/src/main.rs`
- `crates/loopctl/src/client.rs`
- `migrations/0002_add_worktree_provider.sql`

## Verification Checklist
### Implementation Checklist
- [ ] `cargo fmt --check`
- [ ] `cargo test -p loop-core`
- [ ] `cargo test -p loopd`
- [ ] `cargo test -p loopctl`

### Manual QA Checklist (do not mark, human verification)
- [ ]? Run with `worktree_provider=auto` when `wt` is installed
- [ ]? Run with `worktree_provider=worktrunk` when `wt` is missing (expect failure)
- [ ]? Run with `worktree_provider=git` to confirm fallback works

## Notes (Optional)
- Ensure Worktrunk config parsing only reads `worktree-path` and avoids logging secrets.
