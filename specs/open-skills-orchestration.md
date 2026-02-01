# Open Skills Orchestration

**Status:** Draft
**Version:** 1.0
**Last Updated:** 2026-02-01

---

## 1. Overview
### Purpose
Introduce OpenSkills/Agent Skills support to the loop orchestrator by adding a task-aware skill selection step. The orchestrator must choose the next plan task, discover available skills, select relevant skills, and load their instructions into the implementation or review prompt.

### Goals
- Adopt the Agent Skills SKILL.md format and directory conventions (https://agentskills.io/specification).
- Support a built-in skill pack committed to this repo and synced on daemon start.
- Add a plan-aware task selector that chooses the next unchecked task for the run.
- Discover installed skills, extract metadata (name, description, etc.), and generate an available-skills table.
- Select and load a small set of skills into implementation or review prompts based on task relevance.
- Record skill discovery/selection decisions as structured events for observability.

### Non-Goals
- Implement a skills marketplace, installer, or CLI (OpenSkills remains external).
- Execute skill scripts automatically; skills provide instructions only.
- Replace the existing reviewer/implementation agent model.
- Change run storage layout or add new APIs beyond internal prompt changes.

---

## 2. Architecture
### Components
- Built-in Skill Pack: skills committed in-repo (e.g., `skills/`) and synced to a daemon data directory on startup.
- Skill Catalog: scans the configured skill directories, parses SKILL.md frontmatter, and returns a list of available skills.
- Plan Task Selector: parses the plan file and selects the next unchecked task (ignoring verification and manual QA items).
- Skill Matcher: scores skills against the chosen task and selects up to N skills for implementation or review.
- Skill Loader: reads SKILL.md contents for selected skills and formats them for prompt inclusion.
- Prompt Builder Integration: injects available skills and loaded skills into prompts built in `crates/loopd/src/lib.rs` (`build_implementation_prompt`, `build_review_prompt`).

### Dependencies
- YAML frontmatter parsing for SKILL.md (name, description, optional fields).
- No external network calls; skills are loaded from local directories.

### Module/Folder Layout
- `crates/loop-core/src/skills.rs`: core types and parsing utilities.
- `crates/loopd/src/skills/`: catalog, matching, and prompt rendering.
- `crates/loopd/src/lib.rs`: prompt integration hooks.
- `skills/`: built-in skills committed to this repo (optional).

---

## 3. Data Model
### Core Types
- `SkillMetadata`
  - `name`: string (Agent Skills constraints: 1-64 chars, lowercase letters/numbers/hyphens, no leading/trailing hyphen, no consecutive hyphens).
  - `description`: string (1-1024 chars, describes what the skill does and when to use it).
  - `license`: optional string.
  - `compatibility`: optional string.
  - `metadata`: optional map<string, string>.
  - `allowed_tools`: optional list<string> parsed from space-delimited field.
  - `path`: absolute path to skill directory.
  - `location`: enum `project` | `global` (derived from search path).
- `SkillSelection`
  - `run_id`: UUID.
  - `step_kind`: `implementation` | `review`.
  - `task_label`: string (task text from plan).
  - `skills`: list of `{ name, reason }`.
  - `strategy`: `hint` | `match` | `none`.
  - `errors`: list of parse/load errors (if any).

### Skill Directory Layout
- Required: `SKILL.md` with YAML frontmatter (`name`, `description`) and Markdown body instructions.
- Optional: `references/` for detailed docs, `scripts/` for executable helpers, `assets/` for templates.
- Use the Agent Skills template format; the orchestrator extracts the frontmatter fields as the skill title/description source of truth.

### Storage Schema
- No new database tables. Persist selection decisions as events with payloads:
  - `SKILLS_DISCOVERED`: `{ count, locations, names }`
  - `SKILLS_SELECTED`: `SkillSelection` payload
  - `SKILLS_LOAD_FAILED`: `{ name, error }`

---

## 4. Interfaces
### Public APIs
- No new HTTP endpoints.
- New config fields in `crates/loop-core/src/config.rs`:
  - `skills_enabled` (bool, default false).
  - `skills_builtin_dir` (PathBuf, default `skills/` relative to workspace root).
  - `skills_sync_dir` (PathBuf, default `~/.local/share/loopd/skills`).
  - `skills_sync_on_start` (bool, default true).
  - `skills_dirs` (list<PathBuf>, default order matches OpenSkills: `.agent/skills`, `~/.agent/skills`, `.claude/skills`, `~/.claude/skills`).
  - `skills_max_selected_impl` (u8, default 2).
  - `skills_max_selected_review` (u8, default 1).
  - `skills_load_references` (bool, default false) to control optional references/ inclusion.
  - `skills_max_body_chars` (usize, default 20000) to cap loaded SKILL.md content.

### Internal APIs
- `discover_skills(config) -> Vec<SkillMetadata>`.
- `select_task(plan_path) -> Option<TaskSelection>`.
- `select_skills(task, skills, step_kind, limits) -> SkillSelection`.
- `render_available_skills(skills) -> String` (matches OpenSkills XML block format).
- `load_skill_body(skill, include_references) -> String` (mirrors OpenSkills `read` output format).

### Events (names + payloads)
- `SKILLS_DISCOVERED`: `{ run_id, count, locations, names }`.
- `SKILLS_SELECTED`: `SkillSelection`.
- `SKILLS_LOAD_FAILED`: `{ run_id, name, error }`.
- `SKILLS_TRUNCATED`: `{ run_id, name, max_chars }`.

---

## 5. Workflows
### Main Flow
1. On daemon start, sync built-in skills from `skills_builtin_dir` to `skills_sync_dir` (if enabled).
2. Load plan file and pick the next unchecked task (ignore verification checklist items and `[ ]?` items).
3. Discover skills from configured directories and parse SKILL.md frontmatter.
4. If the chosen task includes a skill hint, select those skills first; then fill remaining slots with heuristic matches by keywords in name/description.
5. Build the prompt:
   - Include `available_skills` XML block (OpenSkills format).
   - Append loaded skill bodies for selected skills (OpenSkills `read` format, including `Reading:` and `Base directory:` lines).
   - Include the selected task text in the task instructions section.
6. Execute the implementation or review step and log selection events.

### Edge Cases
- Invalid SKILL.md frontmatter: skip skill, emit `SKILLS_LOAD_FAILED`.
- Duplicate skill names across directories: prefer first match in configured search order.
- Missing or empty description: skip skill (violates spec).
- Plan task references unknown skills: record error and continue without that skill.
- Skill body exceeds size budget: truncate with a warning and log `SKILLS_TRUNCATED`.
- Built-in sync failure: log and continue using the repo directory directly.

### Retry/Backoff
- None. File IO errors are logged and the run continues without the failed skill.

---

## 6. Error Handling
### Error Types
- Frontmatter parse errors.
- Skill file missing or unreadable.
- Plan parse failures.

### Recovery Strategy
- Log and continue with a reduced skill set (strict validation, skip invalid skills).
- If the plan cannot be parsed, fall back to current behavior (agent selects task from plan).

---

## 7. Observability
### Logs
- Log skill discovery counts and selection decisions (name + reason).
- Log parse/IO failures with the skill path.

### Metrics
- `skills_discovered_total`
- `skills_selected_total`
- `skills_load_failed_total`
- `skills_truncated_total`

---

## 8. Security and Privacy
### AuthZ/AuthN
- No changes.

### Data Handling
- Skills are read from local disk only.
- Do not execute skill scripts automatically; only load instructions into prompt context.
- Built-in skills are synced to the daemon data directory; the repo remains the source of truth.

---

## 9. Migration or Rollout
### Compatibility Notes
- Feature is opt-in via `skills_enabled`.
- No schema migrations required.

### Rollout Plan
1. Add skill parsing and catalog with tests.
2. Add task selection and skill matching.
3. Integrate prompt rendering and events.
4. Enable via config in a single workspace.

---

## 10. Open Questions
- None.
