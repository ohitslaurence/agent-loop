# Daemon Review API Implementation Plan

Reference: [daemon-review-api.md](../daemon-review-api.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

---

## Phase 1: Database Schema

- [ ] Add migration `003_review_fields.sql` with new columns (See §9)
- [ ] Add `ReviewStatus` enum to `crates/loop-core/src/types.rs` (See §3)
- [ ] Add review fields to `Run` struct: `review_status`, `review_action_at`, `pr_url`, `merge_commit`
- [ ] Update `Storage::get_run` to read new fields
- [ ] Update `Storage::create_run` to initialize `review_status = pending`
- [ ] Add `Storage::update_review_status` method
- [ ] Run migration and verify schema

---

## Phase 2: Git Diff Helpers

- [ ] Create `crates/loopd/src/handlers/review.rs` module
- [ ] Add `DiffFile`, `DiffCommit`, `DiffStats`, `RunDiffResponse` structs (See §3)
- [ ] Implement `parse_diff_stat` - parse `--stat` output for additions/deletions
- [ ] Implement `parse_diff_patch` - parse unified diff into `DiffFile` structs
- [ ] Implement `get_commits` - run `git log base..head` and parse output
- [ ] Implement `get_commit_diff` - run `git show <sha>` and parse
- [ ] Implement `get_aggregate_diff` - run `git diff base...head` and parse
- [ ] Add unit tests for diff parsing

---

## Phase 3: GET /runs/{id}/diff Endpoint

- [ ] Add `get_run_diff` handler in `handlers/review.rs` (See §4)
- [ ] Fetch run from storage, verify worktree info exists
- [ ] Call git helpers to build `RunDiffResponse`
- [ ] Add route `GET /runs/:id/diff` in `server.rs`
- [ ] Add error handling for missing branch, git failures
- [ ] Test endpoint manually with curl

---

## Phase 4: POST /runs/{id}/scrap Endpoint

- [ ] Add `scrap_run` handler in `handlers/review.rs` (See §4)
- [ ] Verify run status is Completed or Failed
- [ ] Execute `git branch -D run_branch` in workspace
- [ ] Update run with `review_status = scrapped`
- [ ] Add route `POST /runs/:id/scrap` in `server.rs`
- [ ] Test endpoint manually

---

## Phase 5: POST /runs/{id}/merge Endpoint

- [ ] Add `MergeRequest` struct for optional body (strategy field)
- [ ] Add `merge_run` handler in `handlers/review.rs` (See §4)
- [ ] Verify run status is Completed
- [ ] Determine target branch (merge_target_branch or base_branch)
- [ ] Implement squash merge: `git checkout target && git merge --squash && git commit`
- [ ] Implement regular merge: `git merge --no-edit`
- [ ] Capture commit SHA from git output
- [ ] Update run with `review_status = merged`, `merge_commit = sha`
- [ ] Handle merge conflicts (abort and return 409)
- [ ] Restore original branch after merge
- [ ] Add route `POST /runs/:id/merge` in `server.rs`
- [ ] Test endpoint manually

---

## Phase 6: POST /runs/{id}/create-pr Endpoint

- [ ] Add `CreatePrRequest` struct for optional body (title, body fields)
- [ ] Add `create_pr` handler in `handlers/review.rs` (See §4)
- [ ] Verify run status is Completed
- [ ] Check `gh` CLI availability with `which gh`
- [ ] Build default title from run name
- [ ] Build default body with run metadata
- [ ] Execute `gh pr create --head run_branch --base target --title "..." --body "..."`
- [ ] Parse PR URL from stdout
- [ ] Update run with `review_status = pr_created`, `pr_url = url`
- [ ] Add route `POST /runs/:id/create-pr` in `server.rs`
- [ ] Test endpoint manually

---

## Phase 7: Dashboard Integration

- [ ] Unblock Phase 7-9 in `specs/planning/dashboard-plan.md`
- [ ] Test full flow: dashboard → daemon → git

---

## Files to Create

- `crates/loopd/migrations/003_review_fields.sql`
- `crates/loopd/src/handlers/review.rs`

## Files to Modify

- `crates/loop-core/src/types.rs` - add ReviewStatus, review fields to Run
- `crates/loopd/src/storage.rs` - read/write review fields
- `crates/loopd/src/server.rs` - add routes
- `crates/loopd/src/handlers/mod.rs` - export review module
- `crates/loopd/src/git.rs` - may add helpers or reuse existing

---

## Verification Checklist

### Implementation Checklist
- [ ] `cargo build` succeeds
- [ ] `cargo test` passes
- [ ] `cargo clippy` has no warnings
- [ ] Database migration applies cleanly
- [ ] `curl GET /runs/{id}/diff` returns valid JSON
- [ ] `curl POST /runs/{id}/scrap` deletes branch
- [ ] `curl POST /runs/{id}/merge` creates merge commit
- [ ] `curl POST /runs/{id}/create-pr` creates PR (requires gh auth)

### Manual QA Checklist (do not mark—human verification)
- [ ]? Dashboard diff viewer shows commits and files
- [ ]? Dashboard scrap action works
- [ ]? Dashboard merge action works
- [ ]? Dashboard create PR action works

---

## Notes

- Phase 2: Diff parsing is the trickiest part - git output format varies. Use `--format=` flags for predictable output.
- Phase 5: Merge can leave repo in dirty state on conflict - ensure cleanup.
- Phase 6: `gh` CLI must be authenticated. Dashboard should show helpful error if not.
- Phase 7: Once daemon endpoints work, dashboard Phase 7-9 can proceed.
