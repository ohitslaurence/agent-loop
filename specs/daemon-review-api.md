# Daemon Review API

**Status:** Planned
**Version:** 1.0
**Last Updated:** 2026-02-01

---

## 1. Overview

### Purpose
Extend loopd with endpoints to support the dashboard review workflow: viewing diffs, and taking action on completed runs (scrap, merge, create PR).

### Goals
- Expose git diff data for completed runs via REST API
- Enable branch lifecycle actions (delete, merge, create PR) from the dashboard
- Track review status on runs for visibility

### Non-Goals
- Inline diff comments or annotations (future: review-to-agent handoff)
- Multi-repo support
- Conflict resolution UI

### Related Specs
- `specs/dashboard.md` §11 - consumer requirements
- `specs/orchestrator-daemon.md` - existing daemon architecture

---

## 2. Architecture

### Components
```
┌─────────────────────────────────────────────────────────┐
│                    Dashboard (SPA)                       │
│         GET /runs/{id}/diff                              │
│         POST /runs/{id}/scrap|merge|create-pr            │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    loopd daemon                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Review Handlers                     │    │
│  │  get_diff() → git diff/log commands             │    │
│  │  scrap()    → git branch -D                     │    │
│  │  merge()    → git merge --squash                │    │
│  │  create_pr()→ gh pr create                      │    │
│  └─────────────────────────────────────────────────┘    │
│                         │                                │
│                         ▼                                │
│  ┌─────────────────────────────────────────────────┐    │
│  │              Storage                             │    │
│  │  runs.review_status, runs.pr_url, etc.          │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

### Dependencies
- `git` CLI (for diff, log, branch, merge)
- `gh` CLI (for create-pr only, optional)

### Module Layout
```
crates/loopd/src/
  server.rs          # add new routes
  handlers/
    review.rs        # NEW: review endpoint handlers
  git.rs             # extend with diff/merge helpers
```

---

## 3. Data Model

### New Run Fields

Add to `runs` table in `crates/loopd/src/storage.rs`:

```sql
ALTER TABLE runs ADD COLUMN review_status TEXT;      -- pending|reviewed|scrapped|merged|pr_created
ALTER TABLE runs ADD COLUMN review_action_at INTEGER; -- epoch ms
ALTER TABLE runs ADD COLUMN pr_url TEXT;              -- nullable
ALTER TABLE runs ADD COLUMN merge_commit TEXT;        -- nullable
```

### Rust Types

```rust
// crates/loop-core/src/types.rs

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    Pending,
    Reviewed,
    Scrapped,
    Merged,
    PrCreated,
}

// Add to Run struct:
pub review_status: ReviewStatus,
pub review_action_at: Option<DateTime<Utc>>,
pub pr_url: Option<String>,
pub merge_commit: Option<String>,
```

### API Response Types

```rust
// crates/loopd/src/handlers/review.rs

#[derive(Serialize)]
pub struct DiffFile {
    pub path: String,
    pub status: String,       // "added" | "modified" | "deleted" | "renamed"
    pub old_path: Option<String>,
    pub patch: String,        // unified diff for this file
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Serialize)]
pub struct DiffCommit {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,    // ISO 8601
    pub files: Vec<DiffFile>,
    pub stats: DiffStats,
}

#[derive(Serialize)]
pub struct DiffStats {
    pub additions: u32,
    pub deletions: u32,
    pub files_changed: Option<u32>,
}

#[derive(Serialize)]
pub struct RunDiffResponse {
    pub base_ref: String,
    pub head_ref: String,
    pub commits: Vec<DiffCommit>,
    pub files: Vec<DiffFile>,    // aggregate
    pub stats: DiffStats,
}

#[derive(Serialize)]
pub struct MergeResponse {
    pub commit: String,
}

#[derive(Serialize)]
pub struct CreatePrResponse {
    pub url: String,
}
```

---

## 4. Interfaces

### New Endpoints

#### GET /runs/{id}/diff

Returns commits and aggregate diff between base_branch and run_branch.

**Path Parameters:**
- `id` - run UUID

**Response (200 OK):**
```json
{
  "base_ref": "main",
  "head_ref": "run/dashboard-navigator",
  "commits": [
    {
      "sha": "abc123def456",
      "message": "Add user authentication",
      "author": "loop-agent",
      "timestamp": "2026-02-01T10:00:00Z",
      "files": [
        {
          "path": "src/auth.ts",
          "status": "added",
          "patch": "diff --git a/src/auth.ts...",
          "additions": 42,
          "deletions": 0
        }
      ],
      "stats": { "additions": 42, "deletions": 0 }
    }
  ],
  "files": [
    {
      "path": "src/auth.ts",
      "status": "added",
      "patch": "diff --git a/src/auth.ts...",
      "additions": 42,
      "deletions": 0
    }
  ],
  "stats": { "additions": 42, "deletions": 10, "files_changed": 5 }
}
```

**Errors:**
- 404: Run not found
- 400: Run has no worktree info (branch unknown)
- 500: Git command failed

**Implementation:**
```rust
// In workspace_root:
// 1. git log base_branch..run_branch --format="%H|%s|%an|%aI"
// 2. For each commit: git show <sha> --stat --patch
// 3. git diff base_branch...run_branch --stat --patch (aggregate)
```

---

#### POST /runs/{id}/scrap

Delete the run branch and mark run as scrapped.

**Path Parameters:**
- `id` - run UUID

**Response (204 No Content)**

**Side Effects:**
- Deletes branch `run_branch` from the repository
- Sets `review_status = 'scrapped'`
- Sets `review_action_at = now()`

**Errors:**
- 404: Run not found
- 400: Run not in completed/failed state
- 400: Branch doesn't exist
- 500: Git command failed

**Implementation:**
```rust
// In workspace_root:
// 1. Verify run.status in [Completed, Failed]
// 2. git branch -D run_branch
// 3. Update run in storage
```

---

#### POST /runs/{id}/merge

Merge run_branch into merge_target_branch (or base_branch if not set).

**Path Parameters:**
- `id` - run UUID

**Request Body (optional):**
```json
{
  "strategy": "squash"  // "squash" (default) | "merge"
}
```

**Response (200 OK):**
```json
{
  "commit": "abc123def456789..."
}
```

**Side Effects:**
- Merges run_branch into target branch
- Sets `review_status = 'merged'`
- Sets `review_action_at = now()`
- Sets `merge_commit = <commit sha>`

**Errors:**
- 404: Run not found
- 400: Run not in completed state
- 400: Branch doesn't exist
- 409: Merge conflict (returns conflict details)
- 500: Git command failed

**Implementation:**
```rust
// In workspace_root:
// 1. Verify run.status == Completed
// 2. target = merge_target_branch || base_branch
// 3. git checkout target
// 4. If squash: git merge --squash run_branch && git commit -m "Merge run/{name}"
// 5. Else: git merge run_branch --no-edit
// 6. Capture commit SHA
// 7. Update run in storage
// 8. git checkout - (return to previous branch)
```

---

#### POST /runs/{id}/create-pr

Create a GitHub PR from run_branch to merge_target_branch.

**Path Parameters:**
- `id` - run UUID

**Request Body (optional):**
```json
{
  "title": "Feature: Add user authentication",
  "body": "## Summary\n- Added login flow\n- Added session middleware"
}
```

**Response (200 OK):**
```json
{
  "url": "https://github.com/owner/repo/pull/123"
}
```

**Side Effects:**
- Creates PR via `gh pr create`
- Sets `review_status = 'pr_created'`
- Sets `review_action_at = now()`
- Sets `pr_url = <PR URL>`

**Errors:**
- 404: Run not found
- 400: Run not in completed state
- 400: Branch doesn't exist
- 503: `gh` CLI not available
- 500: `gh pr create` failed (returns stderr)

**Implementation:**
```rust
// In workspace_root:
// 1. Verify run.status == Completed
// 2. Verify gh CLI available: which gh
// 3. target = merge_target_branch || base_branch
// 4. Default title = run.name
// 5. Default body = "Created by loopd run {id}"
// 6. gh pr create --head run_branch --base target --title "..." --body "..."
// 7. Parse PR URL from stdout
// 8. Update run in storage
```

---

## 5. Workflows

### Get Diff Flow
```
Dashboard requests GET /runs/{id}/diff
  → Handler fetches run from storage
  → Verify run has worktree info (base_branch, run_branch)
  → Execute git log to get commits
  → Execute git show per commit for per-commit diffs
  → Execute git diff for aggregate diff
  → Parse diff output into structured response
  → Return JSON
```

### Scrap Flow
```
Dashboard requests POST /runs/{id}/scrap
  → Handler fetches run from storage
  → Verify run.status in [Completed, Failed]
  → Execute git branch -D run_branch
  → Update run: review_status=scrapped, review_action_at=now
  → Return 204
```

### Merge Flow
```
Dashboard requests POST /runs/{id}/merge
  → Handler fetches run from storage
  → Verify run.status == Completed
  → Determine target branch
  → Execute git checkout target
  → Execute git merge (squash or regular)
  → Capture commit SHA
  → Update run: review_status=merged, merge_commit=sha
  → Restore previous branch
  → Return { commit }
```

### Create PR Flow
```
Dashboard requests POST /runs/{id}/create-pr
  → Handler fetches run from storage
  → Verify run.status == Completed
  → Verify gh CLI available
  → Determine target branch and PR metadata
  → Execute gh pr create
  → Parse PR URL from output
  → Update run: review_status=pr_created, pr_url=url
  → Return { url }
```

---

## 6. Error Handling

### Error Types

| Error | HTTP Status | Response |
|-------|-------------|----------|
| Run not found | 404 | `{ "error": "run not found: {id}" }` |
| Invalid state | 400 | `{ "error": "run must be completed to merge" }` |
| Branch missing | 400 | `{ "error": "branch not found: {branch}" }` |
| Merge conflict | 409 | `{ "error": "merge conflict", "files": [...] }` |
| gh CLI missing | 503 | `{ "error": "gh CLI not available" }` |
| Git failed | 500 | `{ "error": "git command failed: {stderr}" }` |

### Recovery Strategy
- All git operations should be atomic where possible
- On merge conflict: abort merge, return conflict info
- On any failure: ensure working directory is clean (git reset if needed)

---

## 7. Observability

### Logs
- Log each review action with run_id, action, result
- Log git command execution time
- Log gh CLI output on failure

### Metrics (future)
- `loopd_review_actions_total{action=scrap|merge|pr}` - counter
- `loopd_review_action_duration_ms{action}` - histogram

---

## 8. Security and Privacy

### AuthZ/AuthN
- Uses existing loopd auth (LOOPD_AUTH_TOKEN if set)
- All endpoints require same auth as other run endpoints

### Data Handling
- Diff content may contain sensitive code - same trust model as existing log endpoints
- PR creation uses local gh CLI auth - no token handling in loopd

---

## 9. Migration

### Database Migration

```sql
-- migrations/003_review_fields.sql
ALTER TABLE runs ADD COLUMN review_status TEXT DEFAULT 'pending';
ALTER TABLE runs ADD COLUMN review_action_at INTEGER;
ALTER TABLE runs ADD COLUMN pr_url TEXT;
ALTER TABLE runs ADD COLUMN merge_commit TEXT;
```

### Rollout Plan
1. Add database migration
2. Add types to loop-core
3. Add git helpers (diff parsing, merge)
4. Add handlers
5. Add routes
6. Update storage layer
7. Test with dashboard

---

## 10. Open Questions

1. **Diff size limits** - Should we paginate or truncate very large diffs?
   - Start without limits, add if needed

2. **Branch protection** - Should we check for branch protection rules before merge?
   - No, let git/gh fail naturally

3. **Push after merge** - Should merge endpoint also push to remote?
   - No, keep it local. User can push manually or via separate endpoint.

4. **PR draft mode** - Should create-pr support draft PRs?
   - Add `draft: bool` option to request body (future)
