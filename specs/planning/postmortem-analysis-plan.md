# Run Postmortem and Summary Artifacts Implementation Plan

Reference: [postmortem-analysis.md](../postmortem-analysis.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

## Phase 1: Config + Interfaces
- [x] Add `postmortem` and `summary_json` config keys (See §4)
- [x] Add `loopctl analyze` command with prompt-only mode (See §4)
- [ ] Add HTTP endpoint `POST /runs/{id}/postmortem` (See §4)

## Phase 2: Summary JSON Writer
- [x] Implement summary.json generation in loopd using schema from spec (See §3, §5)
- [x] Register summary.json in artifact storage (See §3)

## Phase 3: Postmortem Pipeline
- [x] Implement analysis prompts and artifact layout (See §3, §5)
- [x] Run claude analysis steps and write outputs (See §5)
- [ ] Emit POSTMORTEM_START/END events (See §4, §7)

## Phase 4: Docs + Compatibility
- [ ] Update README/ARCHITECTURE for daemon postmortem parity (See §9)
- [ ] Note deprecation path for `bin/loop-analyze` (See §9)

## Files to Create
- `crates/loopd/src/postmortem.rs`
- `specs/postmortem-analysis.md`
- `specs/planning/postmortem-analysis-plan.md`

## Files to Modify
- `crates/loop-core/src/config.rs`
- `crates/loop-core/src/events.rs`
- `crates/loopd/src/lib.rs`
- `crates/loopd/src/storage.rs`
- `crates/loopd/src/server.rs`
- `crates/loopctl/src/main.rs`
- `specs/README.md`

## Verification Checklist
### Implementation Checklist
- [R] `cargo fmt --check`
- [ ] `cargo test -p loop-core`
- [ ] `cargo test -p loopd`
- [R] `cargo test -p loopctl`

### Manual QA Checklist (do not mark—human verification)
- [ ]? Run a daemon loop to completion and confirm `summary.json` exists
- [ ]? Run `loopctl analyze --prompt-only` and confirm prompt generation
- [ ]? Run `loopctl analyze` and confirm analysis reports are created

## Notes (Optional)
- Analysis should be best-effort; failures must not change run status.
