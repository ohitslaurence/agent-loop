# Open Skills Orchestration Implementation Plan

Reference: [open-skills-orchestration.md](../open-skills-orchestration.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

## Phase 1: Skill catalog + metadata parsing
- [x] Add core skill types and frontmatter parsing utilities (see §2.1, §3.1, §4.2).
- [x] Add built-in skill sync from repo `skills/` to daemon data directory (see §2.1, §4.1, §5.1).
- [x] Implement directory scanning with OpenSkills priority order and deduping (see §4.1, §5.1).
- [x] Add unit tests for name/description validation and parsing failures (see §3.1, §6.1).

## Phase 2: Plan task selection + skill matching
- [x] Parse plan checkbox tasks and select the next unchecked task (ignore verification and `[ ]?` items) (see §5.1).
- [x] Add optional skill hint parsing in plan task text (see §5.2, §10).
- [x] Implement keyword-based matching for skill selection with per-step limits, filling after hints (see §4.1, §5.1).
- [x] Add tests for plan parsing and matching behavior (see §6.1).

## Phase 3: Prompt integration + skill loading
- [ ] Render `available_skills` XML block in prompts (OpenSkills format) (see §4.2, §5.1).
- [ ] Load selected skills into prompt using OpenSkills `read` output format (see §4.2, §5.1).
- [ ] Enforce skill body size cap with `SKILLS_TRUNCATED` event (see §4.3, §5.2).
- [ ] Inject selected task text into implementation and review prompts (see §5.1).
- [ ] Emit `SKILLS_DISCOVERED`/`SKILLS_SELECTED` events during run execution (see §4.3, §7.1).

## Phase 4: Config + observability
- [x] Add config fields for skills enablement, builtin sync, and limits, including max body chars (see §4.1).
- [ ] Add logs and metrics for discovery/selection failures (see §7.1, §7.2).

## Files to Create
- `crates/loop-core/src/skills.rs`
- `crates/loopd/src/skills/mod.rs`
- `crates/loopd/src/skills/catalog.rs`
- `crates/loopd/src/skills/match.rs`
- `skills/`

## Files to Modify
- `crates/loop-core/src/config.rs`
- `crates/loopd/src/lib.rs`
- `crates/loopd/src/storage.rs`
- `crates/loopd/src/events.rs`
- `crates/loopd/tests/server_integration.rs`

## Verification Checklist
### Implementation Checklist
- [R] `cargo test -p loop-core`
- [R] `cargo test -p loopd`

### Manual QA Checklist (do not mark—human verification)
- [ ]? Run a loop with `skills_enabled=true` and confirm prompt includes selected skill instructions.

## Notes (Optional)
- Phase 2: If plan parsing is ambiguous, fall back to existing agent-chosen task behavior.
