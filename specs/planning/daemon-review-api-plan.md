# Daemon Review API Implementation Plan

Reference: [daemon-review-api.md](../daemon-review-api.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

## Plan Guidelines
- Each phase should be completable in <5 minutes
- ONE commit per phase
- Complete one phase, commit, then move to the next

---

## Phase 1a: Database Migration

- [x] Add migration file for review fields (See §9)
- [x] Columns: `review_status TEXT`, `review_action_at INTEGER`, `pr_url TEXT`, `merge_commit TEXT`

---

## Phase 1b: ReviewStatus Type

- [x] Add `ReviewStatus` enum to `crates/loop-core/src/types.rs` (See §3)
- [x] Variants: `Pending`, `Reviewed`, `Scrapped`, `Merged`, `PrCreated`
- [x] Derive `Serialize`, `Deserialize`, `Default` (default = Pending)

---

## Phase 1c: Run Struct Fields

- [x] Add review fields to `Run` struct in `crates/loop-core/src/types.rs`
- [x] Fields: `review_status: ReviewStatus`, `review_action_at: Option<DateTime<Utc>>`, `pr_url: Option<String>`, `merge_commit: Option<String>`

---

## Phase 1d: Storage Read

- [x] Update `Storage::get_run` in `crates/loopd/src/storage.rs` to read new fields
- [x] Update `Storage::list_runs` to read new fields

---

## Phase 1e: Storage Write

- [x] Update `Storage::create_run` to initialize `review_status = pending`
- [x] Add `Storage::update_review_status` method

---

## Phase 2a: Review Module Setup

- [ ] Create `crates/loopd/src/handlers/review.rs` module
- [ ] Add `DiffFile`, `DiffCommit`, `DiffStats`, `RunDiffResponse` structs (See §3)
- [ ] Export module in `handlers/mod.rs`

---

## Phase 2b: Diff Parsing Helpers

- [ ] Implement `parse_numstat` - parse `git diff --numstat` for additions/deletions per file
- [ ] Implement `get_file_patch` - get unified diff for a single file

---

## Phase 2c: Commit List Helper

- [ ] Implement `get_commits` - run `git log base..head --format="%H|%s|%an|%aI"` and parse

---

## Phase 2d: Aggregate Diff Helper

- [ ] Implement `get_aggregate_diff` - run `git diff base...head` and parse into `RunDiffResponse`

---

## Phase 3: GET /runs/{id}/diff Endpoint

- [ ] Add `get_run_diff` handler in `handlers/review.rs` (See §4)
- [ ] Add route `GET /runs/:id/diff` in `server.rs`
- [ ] Return 404 if run not found, 400 if no worktree info

---

## Phase 4: POST /runs/{id}/scrap Endpoint

- [ ] Add `scrap_run` handler - verify status, run `git branch -D`, update storage (See §4)
- [ ] Add route `POST /runs/:id/scrap` in `server.rs`

---

## Phase 5a: Merge Handler Setup

- [ ] Add `MergeRequest` struct with optional `strategy` field
- [ ] Add `MergeResponse` struct with `commit` field
- [ ] Add `merge_run` handler skeleton in `handlers/review.rs`

---

## Phase 5b: Merge Implementation

- [ ] Implement squash merge: checkout target, merge --squash, commit
- [ ] Capture commit SHA, update storage with `review_status = merged`
- [ ] Add route `POST /runs/:id/merge` in `server.rs`

---

## Phase 6: POST /runs/{id}/create-pr Endpoint

- [ ] Add `CreatePrRequest` and `CreatePrResponse` structs
- [ ] Add `create_pr` handler - verify status, check gh, run `gh pr create` (See §4)
- [ ] Parse PR URL from stdout, update storage
- [ ] Add route `POST /runs/:id/create-pr` in `server.rs`

---

## Phase 7: Dashboard Unblock

- [ ] Update `specs/planning/dashboard-plan.md` to unblock Phase 7-9

---

## Files to Create

- `crates/loopd/migrations/0004_review_fields.sql`
- `crates/loopd/src/handlers/review.rs`

## Files to Modify

- `crates/loop-core/src/types.rs` - add ReviewStatus, review fields to Run
- `crates/loopd/src/storage.rs` - read/write review fields
- `crates/loopd/src/server.rs` - add routes
- `crates/loopd/src/handlers/mod.rs` - export review module

---

## Verification Checklist

### Implementation Checklist
- [ ] `cargo build` succeeds
- [ ] `cargo test` passes
- [ ] `cargo clippy` has no warnings

### Manual QA Checklist (do not mark—human verification)
- [ ]? `curl GET /runs/{id}/diff` returns valid JSON
- [ ]? `curl POST /runs/{id}/scrap` deletes branch
- [ ]? `curl POST /runs/{id}/merge` creates merge commit
- [ ]? `curl POST /runs/{id}/create-pr` creates PR

---

## Notes

- Migration filename is `0004_` (0003 already exists per postmortem)
- Use `git diff --numstat` for reliable additions/deletions parsing
- Use `git log --format=` for predictable commit parsing
