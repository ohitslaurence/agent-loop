# Worktrunk Worktree Integration

**Status:** Complete
**Version:** 1.0
**Last Updated:** 2026-01-30

---

## 1. Overview
### Purpose
Integrate Worktrunk as a first-class worktree provider so loop runs can automatically create, run inside, and optionally clean up worktrees using Worktrunk's workflow and hooks, while preserving a git-native fallback.

### Goals
- Use Worktrunk for worktree lifecycle when available (create, switch, remove).
- Keep git-native worktree support as a fallback provider.
- Preserve existing run branch and merge semantics defined in `specs/orchestrator-daemon.md`.
- Ensure worktree path resolution is deterministic and matches Worktrunk config.
- Make provider selection explicit and observable in run metadata and events.

### Non-Goals
- Implement Worktrunk features beyond worktree lifecycle (LLM commit, merge workflow).
- Support remote/multi-host worktree orchestration.
- Replace Worktrunk's own config or hook system.

---

## 2. Architecture
### Components
- Worktree provider interface in `crates/loopd/src/worktree.rs`.
- Worktrunk provider implementation in `crates/loopd/src/worktree_worktrunk.rs`.
- Git provider implementation reusing `crates/loopd/src/git.rs`.
- Config extensions in `crates/loop-core/src/config.rs`.

### Dependencies
- External: `wt` (Worktrunk CLI) for provider=worktrunk.
- Existing: `git` CLI for provider=git.

### Module/Folder Layout
```
crates/loopd/
  src/worktree.rs
  src/worktree_worktrunk.rs
  src/git.rs
```

---

## 3. Data Model
### Core Types
- WorktreeProvider: auto, worktrunk, git.
- RunWorktree: add `provider` field to `crates/loop-core/src/types.rs`.

### Storage Schema
Persist provider for auditability:
- `runs.worktree_provider` TEXT (auto|worktrunk|git).
Track cleanup state:
- `runs.worktree_cleanup_status` TEXT (`cleaned` | `failed` | `skipped`).
- `runs.worktree_cleaned_at` INTEGER (epoch ms).

---

## 4. Interfaces
### Public APIs
Config keys in `.loop/config` (parsed by `crates/loop-core/src/config.rs`):
- `worktree_provider=auto|worktrunk|git`
- `worktrunk_bin=wt` (path to Worktrunk CLI)
- `worktrunk_config_path=~/.config/worktrunk/config.toml` (optional override)
- `worktrunk_copy_ignored=true|false`

CLI flags in `crates/loopctl/src/main.rs`:
- `loopctl run --worktree-provider auto|worktrunk|git`
- `loopctl run --worktrunk-bin /path/to/wt`
- `loopctl run --worktrunk-config /path/to/config.toml`
- `loopctl run --worktrunk-copy-ignored`

### Internal APIs
- `worktree::resolve_provider(config, workspace_root) -> WorktreeProvider`
- `worktree::prepare(run, config) -> RunWorktree`
- `worktree::cleanup(run, config)`

### Events (names + payloads)
- `WORKTREE_PROVIDER_SELECTED`: {run_id, provider}
- `WORKTREE_CREATED`: {run_id, provider, worktree_path, run_branch}
- `WORKTREE_REMOVED`: {run_id, provider, worktree_path}

---

## 5. Workflows
### Main Flow
```
loopctl run
  -> resolve provider (auto -> worktrunk if available, else git)
  -> create worktree + run branch
  -> execute loop inside worktree path
  -> merge to target branch (git)
  -> optional cleanup
```

### Provider Selection
- `worktree_provider=auto`: use Worktrunk if `wt` is available, else fallback to git.
- `worktree_provider=worktrunk`: fail run if `wt` is not available.
- `worktree_provider=git`: always use `crates/loopd/src/git.rs`.

### Worktrunk Worktree Creation
- Use `wt switch --create <run_branch>` to create and select the worktree.
- Worktree path derived from Worktrunk config `worktree-path` (from `worktrunk_config_path`) or default template in `crates/loopd/src/git.rs` if not found.
- Optional `wt step copy-ignored` when `worktrunk_copy_ignored=true`.

### Cleanup
- If `worktree_cleanup=true` (default), call `wt remove` for Worktrunk provider or `git worktree remove` for git provider.
- Record cleanup status and timestamp on success; record failure status on errors.

### ASCII Diagram
```
run -> provider select -> worktree create -> run steps -> merge -> cleanup
```

---

## 6. Error Handling
### Error Types
- Provider not available (worktrunk requested, `wt` missing).
- Worktrunk command failure (non-zero exit).
- Worktree path resolution failure (invalid config path/template).

### Recovery Strategy
- Auto provider: fallback to git when Worktrunk is missing.
- Hard provider: mark run FAILED with reason and persist event.
- Cleanup failures are logged but do not fail completed runs.

---

## 7. Observability
### Logs
- Log provider selection and worktree creation/cleanup steps with run_id.

### Metrics
- worktree_provider_count{provider}
- worktree_create_duration_ms

---

## 8. Security and Privacy
### AuthZ/AuthN
- No new auth surface; relies on existing localhost API.

### Data Handling
- Respect Worktrunk hook approval behavior; do not bypass prompts.
- Do not log Worktrunk config contents beyond the worktree-path template.

---

## 9. Migration or Rollout
### Compatibility Notes
- Git provider remains default fallback.
- Worktrunk integration is opt-in via config or CLI until stabilized.

### Rollout Plan
1. Add provider selection and metadata fields.
2. Implement Worktrunk provider and config parsing.
3. Add integration tests for provider selection and worktree creation.

---

## 10. Open Questions
- Should Worktrunk be the default provider when detected, or stay opt-in?
- Do we allow Worktrunk hooks to block non-interactive runs?
