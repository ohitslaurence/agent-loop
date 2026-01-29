# Learnings

Repo-wide patterns and lessons learned from implementation reviews.
Reviewers add entries here when they find patterns that apply beyond a single task.
Periodically curate this into proper codebase rules (CLAUDE.md, lint configs, etc).

---

## SQLite Storage: Column Order Matters with `SELECT *`

**Problem:** When using `sqlx::query_as` with `SELECT *`, the struct field order must match the database column order exactly. ALTER TABLE adds columns at the END of the table, not at the position where you define them in the migration.

**Symptom:** Flaky tests with "index out of bounds" errors from `sqlx-sqlite-worker` threads when tests run in parallel.

**Fix:** Use explicit column lists instead of `SELECT *`:
```rust
const RUNS_COLUMNS: &str = "id, name, status, ..., worktree_provider";
let query = format!("SELECT {} FROM runs WHERE id = ?1", RUNS_COLUMNS);
```

**Why:** The struct field order must match the query result column order. With `SELECT *`, the column order depends on the database schema evolution (ALTER TABLE adds at end). Using explicit columns ensures consistency regardless of schema migration history.

---

