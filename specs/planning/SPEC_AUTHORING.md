# Spec Authoring Guide

This guide explains how to add a new spec and implementation plan in this repo.
It is intended for agents who are asked to draft or update specs without any prior context.

## Folder Layout

- `specs/<topic>.md` -- durable system spec
- `specs/planning/<topic>-plan.md` -- implementation plan (phase checklist)
- `specs/README.md` -- index of specs and plans

## Authoring Workflow (Required)

1. Read related specs in `specs/` and reference material in `specs/research/`.
2. Create `specs/<topic>.md` using the required structure below.
3. Create `specs/planning/<topic>-plan.md` with small, checkable phases.
4. Update `specs/README.md` to link the spec and plan.

## Spec Requirements (Strict)

- Use the metadata header exactly as shown.
- Update **Status** and **Last Updated** whenever the spec changes.
- Number sections for citation in the plan (1, 2, 3...).
- Use concrete file paths, types, and example payloads where applicable.
- Include diagrams or data-flow steps for any non-trivial lifecycle.
- Keep the spec stable: no task lists or checkboxes here.
- Write each section so it can be implemented independently; avoid hidden dependencies between sections.
- **Cite existing code** — when pseudocode references utilities, types, or patterns, include the file path (e.g., "use `success()` from `billpay/handlers.ts`"). Agents recreate code when not told where existing code lives.

## Spec Structure (Template)

```
# <Title>

**Status:** Draft | Planned | In Progress | Implemented
**Version:** 1.0
**Last Updated:** YYYY-MM-DD

---

## 1. Overview
### Purpose
### Goals
### Non-Goals

---

## 2. Architecture
### Components
### Dependencies
### Module/Folder Layout

---

## 3. Data Model
### Core Types
### Storage Schema (if any)

---

## 4. Interfaces
### Public APIs
### Internal APIs
### Events (names + payloads)

---

## 5. Workflows
### Main Flow
### Edge Cases
### Retry/Backoff (if any)

---

## 6. Error Handling
### Error Types
### Recovery Strategy

---

## 7. Observability
### Logs
### Metrics
### Traces

---

## 8. Security and Privacy
### AuthZ/AuthN
### Data Handling

---

## 9. Migration or Rollout
### Compatibility Notes
### Rollout Plan

---

## 10. Open Questions
```

Notes:

- Keep sections short but concrete; use tables when listing types or endpoints.
- Add ASCII diagrams for system flows and fan-out patterns.
- Link to existing code where the design integrates.
- Favor self-contained requirements so phases can be executed out of order.
- **Reference existing utilities by path** — when pseudocode uses helpers like `success()`, `failure()`, or shared types, cite where they live (e.g., "use `success()` from `billpay/handlers.ts`"). Agents will recreate utilities if not pointed to existing ones.

## Implementation Plan Structure (Template)

```
# <Title> Implementation Plan

Reference: [<topic>.md](../<topic>.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

## Phase 1: <phase name>
- [ ] Task with citation to spec section (e.g., "See §2.3")

## Phase 2: <phase name> [BLOCKED by: Phase 1]
- [ ] Task with citation to spec section

## Files to Create
- `path/to/file`

## Files to Modify
- `path/to/file`

## Verification Checklist
### Implementation Checklist
- [ ] `command` or agent-verifiable step

### Manual QA Checklist (do not mark—human verification)
- [ ]? Manual QA item

## Notes (Optional)
- Phase X: <note about blockers, edge cases, or follow-ups>
```

Notes:

- Each task must map to a spec section or requirement.
- Keep tasks small enough to complete in a single loop iteration; avoid bundling unrelated changes.
- Do not assume sequential execution; each phase should be independently runnable.
- Order phases by priority/impact, not dependency. The implementing agent may reorder based on impact.
- If a phase depends on another, state it explicitly in the phase heading or first task (use "[BLOCKED by: Phase X]").
- Do not mark a phase complete until all verification steps tied to that phase pass. If the checklist is shared,
  call out which steps apply to each phase in the phase notes.
- Manual QA items must use `[ ]?` and should not be checked by agents. Human reviewers own these items.
- Mark completed work with `[x]` and add notes about commits or tests. Verification checklist items should be
  marked `[R]` when run; do not use `[x]` for verification.
- Required sections: Files to Create, Files to Modify, Verification Checklist (use `None` if empty).
- Add Notes only when there is information useful to future phases (blockers, risks, edge cases, follow-ups).

## Updating the Index

Add a row in `specs/README.md` linking the spec and plan:

```
| [<topic>.md](./<topic>.md) | [<topic>-plan.md](./planning/<topic>-plan.md) | <code path> | <purpose> |
```

If a plan does not exist yet, use `--` in the plan column until it is created.
