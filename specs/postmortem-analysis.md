# Run Postmortem and Summary Artifacts

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-01-30

---

## 1. Overview
### Purpose
Provide deterministic run summaries and automated postmortem analysis for daemon-managed runs, replacing the legacy `bin/loop` + `bin/loop-analyze` workflow.

### Goals
- Generate `summary.json` for every run end (success, failure, cancel), matching legacy schema.
- Optionally run postmortem analysis using Claude, producing Markdown reports in the run directory.
- Allow on-demand postmortem via CLI without requiring the bash loop.
- Keep file names and layouts compatible with existing log tooling.

### Non-Goals
- Experiment-mode analysis and metrics selection (out of scope for v1).
- Web UI or external report publishing.

---

## 2. Architecture
### Components
- **loopd postmortem pipeline**: writes `summary.json` and (if enabled) generates postmortem reports.
- **loopctl analyze**: triggers postmortem generation on demand and can print prompts without running.
- **Storage artifacts**: persist summary + analysis files in run directories.

### Dependencies
- `claude` CLI (required to run analysis; optional for prompt-only mode).
- `git` CLI (optional, used for snapshots; if missing, skip snapshot).

### Module/Folder Layout
```
crates/
  loopd/
    postmortem.rs    # summary.json + analysis pipeline
    runner.rs        # used to execute claude analysis steps
  loopctl/
    main.rs          # analyze command
```

---

## 3. Data Model
### Core Types

**Summary JSON** (write to `<run_dir>/summary.json`). Schema mirrors `write_summary_json()` in `lib/agent-loop-ui.sh`:

| Field | Type | Nullable | Notes |
| --- | --- | --- | --- |
| run_id | string | no | UUIDv7 from `runs.id` |
| start_ms | int | no | run start epoch ms |
| end_ms | int | no | run end epoch ms |
| total_duration_ms | int | no | end_ms - start_ms |
| iterations_run | int | no | total iterations executed |
| completed_iteration | int | yes | iteration id where completion detected |
| avg_duration_ms | int | no | total_duration_ms / iterations_run |
| last_exit_code | int | no | last runner exit code |
| completion_mode | string | yes | `exact` or `trailing` |
| model | string | no | model used for run |
| exit_reason | string | no | `complete_plan`, `complete_reviewer`, `iterations_exhausted`, `claude_failed`, `failed`, `canceled` |
| run_log | string | no | path to run log |
| run_report | string | no | path to report TSV |
| prompt_snapshot | string | no | path to prompt snapshot |
| last_iteration_tail | string | yes | path to last iteration tail |
| last_iteration_log | string | yes | path to last iteration log |

Example:
```json
{
  "run_id": "01HS6Q...",
  "start_ms": 1738218455000,
  "end_ms": 1738219056000,
  "total_duration_ms": 601000,
  "iterations_run": 12,
  "completed_iteration": 11,
  "avg_duration_ms": 50083,
  "last_exit_code": 0,
  "completion_mode": "trailing",
  "model": "opus",
  "exit_reason": "complete_plan",
  "run_log": ".../run.log",
  "run_report": ".../report.tsv",
  "prompt_snapshot": ".../prompt.txt",
  "last_iteration_tail": ".../iter-11.tail.txt",
  "last_iteration_log": ".../iter-11.log"
}
```

**Postmortem artifacts** (written to `<run_dir>/analysis/`):
- `run-quality.md` + `run-quality-prompt.txt`
- `spec-compliance.md` + `spec-compliance-prompt.txt`
- `summary.md` + `summary-prompt.txt`
- `git-status.txt`, `git-last-commit.txt`, `git-last-commit.patch`, `git-diff.patch`

### Storage Schema
No new tables. Store new files under the run directory and register them as artifacts via `crates/loopd/src/storage.rs`.

---

## 4. Interfaces
### Public APIs
CLI:
- `loopctl analyze <run_id> [--model <name>] [--prompt-only] [--log-dir <path>]`
- `loopctl analyze --latest [--model <name>] [--prompt-only] [--log-dir <path>]`

### HTTP API
- `POST /runs/{id}/postmortem` `{ "model": "opus", "prompt_only": false }`
- `GET /runs/{id}/postmortem` → list analysis artifacts (paths + timestamps)

### Config
Add daemon support for keys already used by `bin/loop`:
- `postmortem=true|false` (default: true)
- `summary_json=true|false` (default: true)

### Events (names + payloads)
- `POSTMORTEM_START`: `{ run_id, reason }`
- `POSTMORTEM_END`: `{ run_id, status }` where status is `ok` or `failed`

---

## 5. Workflows
### Main Flow
1. Run ends (completed/failed/canceled).
2. Export `report.tsv` from events/steps into the run directory (best-effort).
3. If `summary_json=true`, write `summary.json` to run directory and mirror to global artifacts.
4. If `postmortem=true` and `claude` is available:
   - Create `<run_dir>/analysis/`.
   - Capture git snapshot files (if git available).
   - Generate analysis prompts (run quality + spec compliance + summary) based on `bin/loop-analyze` prompts.
   - Execute each prompt using `claude -p --dangerously-skip-permissions --model <model>` and write outputs.
   - Emit `POSTMORTEM_END` event on completion.

### Edge Cases
- Missing `claude`: skip analysis, log warning, still write summary.
- Missing report TSV: skip analysis and record error in logs.
- No git repo: skip snapshot files, but continue analysis.

### Retry/Backoff
No retries by default. Postmortem is best-effort and must not block run completion.

---

## 6. Error Handling
### Error Types
- `claude` binary missing
- prompt generation failure
- analysis step exit non-zero
- missing run artifacts (report/prompt)

### Recovery Strategy
- Record failures in logs + `POSTMORTEM_END` event with `status=failed`.
- Do not change run status (run remains completed/failed/canceled).

---

## 7. Observability
### Logs
- `summary.json` write logs include path.
- `POSTMORTEM_START`/`POSTMORTEM_END` recorded in event stream.

### Metrics
None required for v1.

### Traces
None.

---

## 8. Security and Privacy
### AuthZ/AuthN
Daemon endpoints remain localhost-only; respect existing auth token if enabled.

### Data Handling
- Postmortem prompts include repo paths, logs, and diffs; avoid uploading secrets outside local execution.
- Use `claude -p --dangerously-skip-permissions` to match legacy behavior in `bin/loop-analyze`.

---

## 9. Migration or Rollout
### Compatibility Notes
- Maintain file names and layout from `bin/loop`/`bin/loop-analyze` for interoperability.
- Support analyzing legacy log layouts (`logs/loop/run-<id>-report.tsv`) when invoked with `--log-dir`.

### Rollout Plan
1. Add summary.json writer in loopd (parity with `lib/agent-loop-ui.sh`).
2. Add postmortem pipeline and CLI.
3. Deprecate `bin/loop-analyze` once daemon parity is validated.

---

## 10. Open Questions
- Should postmortem run inline or in a background task after run completion?
- Should we add termination metadata to summary.json (`clean_termination`, `review_iterations_used`, etc.)?
