# Dashboard Implementation Plan

Reference: [dashboard.md](../dashboard.md)

## Checkbox Legend
- `[ ]` Pending (blocks completion)
- `[~]` Blocked (blocks completion)
- `[x]` Implemented, awaiting review
- `[R]` Reviewed/verified (non-blocking)
- `[ ]?` Manual QA only (ignored)

---

## Phase 1: Project Scaffold

- [x] Initialize Vite + React + TypeScript project in `apps/dashboard/` (See §2)
- [x] Configure Tailwind CSS with dark mode (class strategy)
- [x] Install and configure shadcn/ui (init with default theme)
- [x] Install Tanstack Router, set up file-based routing
- [x] Install Tanstack Query, configure QueryClient
- [ ] Create basic layout with header and placeholder content
- [ ] Verify dev server runs on port 3000

---

## Phase 1b: Testing Infrastructure (See §12)

- [x] Install Vitest, @testing-library/react, @testing-library/jest-dom
- [x] Install MSW (msw)
- [x] Install Playwright (@playwright/test)
- [x] Create `vitest.config.ts` with jsdom environment
- [ ] Create `playwright.config.ts` with headless Chrome
- [x] Create `src/mocks/handlers.ts` with basic endpoint stubs
- [x] Create `src/mocks/fixtures.ts` with sample runs, steps, events
- [x] Create `src/mocks/browser.ts` for dev mode
- [x] Create `src/mocks/server.ts` for test mode
- [x] Create `tests/setup.ts` to start MSW before tests
- [ ] Add scripts: `bun test`, `bun run test:e2e`
- [ ] Verify `bun test` runs without errors

---

## Phase 2: API Client & Types

- [x] Create `src/lib/types.ts` with Run, Step, RunEvent, etc. (See §3)
- [x] Create `src/lib/api.ts` with typed REST client (See §4)
- [x] Add health check function and test against running daemon
- [ ] Create `src/hooks/use-runs.ts` with Tanstack Query (5s polling)

---

## Phase 3: Run List View

- [ ] Create `src/routes/index.tsx` - run list page
- [ ] Create `src/components/run-card.tsx` - status badge, name, workspace, timestamps
- [ ] Create `src/components/run-list.tsx` - grid/list of run cards
- [ ] Create `src/components/workspace-switcher.tsx` - dropdown derived from runs
- [ ] Wire workspace filter to URL query param
- [ ] Add loading and error states

---

## Phase 4: Run Detail View

- [ ] Create `src/routes/runs/$runId.tsx` - run detail page
- [ ] Create `src/hooks/use-run.ts` - single run query
- [ ] Create `src/hooks/use-steps.ts` - steps query
- [ ] Create `src/components/run-detail.tsx` - run metadata display
- [ ] Create `src/components/step-timeline.tsx` - phase progression visualization

---

## Phase 5: SSE Streaming

- [ ] Create `src/lib/sse.ts` - SSE manager with reconnection logic (See §4)
- [ ] Create `src/hooks/use-run-events.ts` - event stream hook
- [ ] Create `src/hooks/use-run-output.ts` - output stream hook
- [ ] Create `src/components/log-viewer.tsx` - streaming log display with auto-scroll
- [ ] Wire events to update run/step status in real-time
- [ ] Add connection status indicator (connected/reconnecting)

---

## Phase 6: Lifecycle Checklist

- [ ] Create `src/components/lifecycle-checklist.tsx` - derive steps from events (See §5)
- [ ] Parse events to build lifecycle: started, iterations, review, verification, merge, cleanup
- [ ] Display as vertical checklist with timestamps
- [ ] Show "Ready for review" state for completed runs with branch

---

## Phase 7: Daemon Review API [BLOCKED by: daemon changes]

> Requires new daemon endpoints (See spec §11). These must be implemented in loopd before this phase.

- [~] Implement `GET /runs/{id}/diff` in loopd
- [~] Implement `POST /runs/{id}/scrap` in loopd
- [~] Implement `POST /runs/{id}/merge` in loopd
- [~] Implement `POST /runs/{id}/create-pr` in loopd
- [~] Add `review_status`, `pr_url`, `merge_commit` fields to runs table

---

## Phase 8: Diff Viewer [BLOCKED by: Phase 7]

- [ ] Install `@pierre/diffs` package (See §13)
- [ ] Create `src/routes/runs/$runId.review.tsx` - review page
- [ ] Create `src/hooks/use-run-diff.ts` - fetch commits + aggregate diff from daemon
- [ ] Create `src/components/diff-viewer.tsx` - render diff with @pierre/diffs PatchDiff
- [ ] Create `src/components/commit-list.tsx` - list commits, click to view each
- [ ] Create `src/components/file-list.tsx` - file tree sidebar with stats
- [ ] Add view mode toggle: Commits view vs All Changes (PR-style) view
- [ ] Add diff style toggle: Split (side-by-side) vs Unified (stacked)
- [ ] Wire file selection to update diff viewer

---

## Phase 9: Review Actions [BLOCKED by: Phase 7, Phase 8]

- [ ] Create `src/components/review-actions.tsx` - action buttons
- [ ] Implement Scrap action (delete branch, show confirmation)
- [ ] Implement Merge action (merge to target, show commit)
- [ ] Implement Create PR action (call gh, show PR link)
- [ ] Add confirmation dialogs for destructive actions
- [ ] Update run card to show review status badge

---

## Phase 10: Polish

- [ ] Add global error banner for daemon unavailable
- [ ] Add toast notifications for transient errors
- [ ] Add empty states (no runs, no steps)
- [ ] Responsive layout for mobile/tablet
- [ ] Keyboard navigation (j/k for run list, esc to go back)

---

## Files to Create

- `apps/dashboard/package.json`
- `apps/dashboard/vite.config.ts`
- `apps/dashboard/tailwind.config.ts`
- `apps/dashboard/tsconfig.json`
- `apps/dashboard/index.html`
- `apps/dashboard/src/main.tsx`
- `apps/dashboard/src/routes/__root.tsx`
- `apps/dashboard/src/routes/index.tsx`
- `apps/dashboard/src/routes/runs/$runId.tsx`
- `apps/dashboard/src/lib/types.ts`
- `apps/dashboard/src/lib/api.ts`
- `apps/dashboard/src/lib/sse.ts`
- `apps/dashboard/src/hooks/use-runs.ts`
- `apps/dashboard/src/hooks/use-run.ts`
- `apps/dashboard/src/hooks/use-steps.ts`
- `apps/dashboard/src/hooks/use-run-events.ts`
- `apps/dashboard/src/hooks/use-run-output.ts`
- `apps/dashboard/src/components/run-list.tsx`
- `apps/dashboard/src/components/run-card.tsx`
- `apps/dashboard/src/components/run-detail.tsx`
- `apps/dashboard/src/components/step-timeline.tsx`
- `apps/dashboard/src/components/log-viewer.tsx`
- `apps/dashboard/src/components/workspace-switcher.tsx`
- `apps/dashboard/src/components/lifecycle-checklist.tsx`
- `apps/dashboard/src/components/diff-viewer.tsx`
- `apps/dashboard/src/components/commit-list.tsx`
- `apps/dashboard/src/components/file-list.tsx`
- `apps/dashboard/src/components/review-actions.tsx`
- `apps/dashboard/src/routes/runs/$runId.review.tsx`
- `apps/dashboard/src/hooks/use-run-diff.ts`
- `apps/dashboard/components.json` (shadcn config)
- `apps/dashboard/vitest.config.ts`
- `apps/dashboard/playwright.config.ts`
- `apps/dashboard/src/mocks/handlers.ts`
- `apps/dashboard/src/mocks/fixtures.ts`
- `apps/dashboard/src/mocks/browser.ts`
- `apps/dashboard/src/mocks/server.ts`
- `apps/dashboard/tests/setup.ts`
- `apps/dashboard/tests/e2e/run-list.spec.ts`
- `apps/dashboard/tests/e2e/run-detail.spec.ts`

## Files to Modify

- None (new app - but Phase 7 requires daemon changes, see spec §11)

---

## Verification Checklist

### Implementation Checklist
- [ ] `bun install` completes without errors
- [ ] `bun dev` starts dev server on port 3000
- [ ] `bun run build` produces production bundle
- [ ] `bun run lint` passes (if configured)
- [ ] `bun test` runs component tests against MSW mocks
- [ ] `bun run test:e2e` runs Playwright tests headless

### Manual QA Checklist (do not mark—human verification)
- [ ]? Run list displays runs from daemon
- [ ]? Workspace filter works
- [ ]? Click run navigates to detail view
- [ ]? Log streaming displays output in real-time
- [ ]? Reconnection works after network blip
- [ ]? Dark mode respects system preference
- [ ]? Lifecycle checklist shows correct steps for completed run
- [ ]? Diff viewer displays aggregate changes
- [ ]? Scrap action deletes branch
- [ ]? Merge action merges to target and shows commit
- [ ]? Create PR action creates PR and shows link

---

## Notes

- Phase 1: Using bun as package manager and runtime
- Phase 5: Start with simple pre element for logs; add virtualization in V1 if needed
- Phase 7: Daemon changes should be specced separately (see dashboard.md §11 for requirements)
- Phase 8: Using @pierre/diffs for diff rendering (docs: https://diffs.com/docs, indexed in Nia)
- Phase 10: Consider adding Sonner for toasts (integrates well with shadcn)
