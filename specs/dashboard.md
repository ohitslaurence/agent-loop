# Dashboard

**Status:** Draft
**Version:** 0.1
**Last Updated:** 2026-02-01

---

## 1. Overview

### Purpose
Web-based dashboard for observing loopd runs across multiple workspaces. Provides real-time visibility into run status, step progress, and streaming output.

### Goals
- Multi-workspace run visibility with filtering
- Real-time run status and log streaming via SSE
- Drill-down from run list to run detail with full output
- **Review workflow**: view aggregate diff, take action (scrap/merge/create PR)
- Clear lifecycle checklist showing what happened (worktree merged, cleaned up, branch ready)

### Non-Goals
- V0: No authentication (tailscale-protected deployment)
- V0: No notifications (Slack, desktop)
- V0: No historical analytics or cost tracking
- V0: No inline commenting on diffs (future: review-to-agent handoff)

---

## 2. Architecture

### Components
```
┌─────────────────────────────────────────────────────────┐
│                    Dashboard (SPA)                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐   │
│  │  Router  │  │  Query   │  │  SSE Event Manager   │   │
│  │(Tanstack)│  │(Tanstack)│  │  (reconnect, buffer) │   │
│  └──────────┘  └──────────┘  └──────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    loopd daemon                          │
│                   127.0.0.1:7700                         │
│  REST: /runs, /runs/{id}, /runs/{id}/steps, /worktrees  │
│  SSE:  /runs/{id}/events, /runs/{id}/output             │
└─────────────────────────────────────────────────────────┘
```

### Tech Stack
- **Framework:** React 18+ with TypeScript
- **Routing:** Tanstack Router (type-safe, file-based)
- **Data Fetching:** Tanstack Query (caching, refetch, SSE integration)
- **Styling:** Tailwind CSS
- **Components:** shadcn/ui (Radix primitives)
- **Build:** Vite

### Module/Folder Layout
```
apps/dashboard/
  src/
    routes/
      __root.tsx           # layout with workspace switcher
      index.tsx            # run list (all workspaces or filtered)
      runs/
        $runId.tsx         # run detail with streaming output
        $runId.review.tsx  # diff review and actions
    components/
      run-list.tsx
      run-card.tsx
      run-detail.tsx
      step-timeline.tsx
      lifecycle-checklist.tsx  # completed steps visualization
      log-viewer.tsx           # streaming log display
      diff-viewer.tsx          # git diff display (uses @pierre/diffs)
      commit-list.tsx          # list commits in run branch
      file-list.tsx            # file tree with change stats
      review-actions.tsx       # scrap/merge/create-pr buttons
      workspace-switcher.tsx
    lib/
      api.ts               # REST client (typed)
      sse.ts               # SSE manager with reconnection
      types.ts             # API types mirrored from loopd
    hooks/
      use-runs.ts          # list runs with polling
      use-run.ts           # single run query
      use-steps.ts         # steps for a run
      use-run-events.ts    # SSE event stream
      use-run-output.ts    # SSE output stream
      use-run-diff.ts      # fetch commits + aggregate diff
  tailwind.config.ts
  vite.config.ts
  package.json
```

### Dependencies
- Runtime: loopd daemon (port 7700)
- Dev: Bun 1.0+

### Quick Start (for implementing agent)
```bash
cd apps/dashboard
bun create vite . --template react-ts
bun add @tanstack/react-router @tanstack/react-query
bun add -d tailwindcss postcss autoprefixer
bunx tailwindcss init -p
bunx shadcn@latest init
```

Dev server must run on port 3000:
```json
// vite.config.ts
export default defineConfig({
  server: { port: 3000 }
})
```

---

## 3. Data Model

### Core Types (mirrored from loopd)

```typescript
type RunStatus = "Pending" | "Running" | "Completed" | "Failed" | "Canceled" | "Paused";

type StepPhase = "Implementation" | "Review" | "Verification" | "Watchdog" | "Merge";

type StepStatus = "Pending" | "Running" | "Succeeded" | "Failed";

interface Run {
  id: string;
  name: string;
  name_source: string;
  status: RunStatus;
  workspace_root: string;
  spec_path: string;
  plan_path?: string;
  worktree?: RunWorktree;
  created_at: string;  // ISO 8601
  updated_at: string;
}

interface RunWorktree {
  worktree_path: string;
  run_branch: string;
  base_branch: string;
  merge_target_branch?: string;
  merge_strategy: "None" | "Squash" | "MergeLast";
  provider: "Native" | "Worktrunk";
}

interface Step {
  id: string;
  run_id: string;
  phase: StepPhase;
  status: StepStatus;
  attempt: number;
  started_at?: string;
  completed_at?: string;
  output_path?: string;
  exit_code?: number;
}

interface RunEvent {
  id: string;
  run_id: string;
  step_id?: string;
  event_type: string;
  timestamp: number;  // ms since epoch
  payload: Record<string, unknown>;
}
```

### Review Workflow Types

```typescript
// Lifecycle events derived from run events
interface LifecycleStep {
  label: string;           // e.g., "Worktree created", "Merged to branch"
  completed: boolean;
  timestamp?: string;
}

// Commits in the run branch
interface RunCommit {
  sha: string;
  message: string;
  author: string;
  timestamp: string;
  files: DiffFile[];
  stats: { additions: number; deletions: number };
}

// Aggregate diff for review
interface RunDiff {
  base_ref: string;        // e.g., "main"
  head_ref: string;        // e.g., "loop/run-abc123"
  commits: RunCommit[];    // individual commits for commit view
  files: DiffFile[];       // aggregate for PR view
  stats: { additions: number; deletions: number; files_changed: number };
}

interface DiffFile {
  path: string;
  status: "added" | "modified" | "deleted" | "renamed";
  old_path?: string;       // for renames
  patch: string;           // unified diff for this file (for PatchDiff component)
  additions: number;
  deletions: number;
}

// Review action results
type ReviewAction = "scrapped" | "merged" | "pr_created";

interface ReviewResult {
  action: ReviewAction;
  pr_url?: string;         // if pr_created
  merge_commit?: string;   // if merged
}
```

### Client State
```typescript
interface DashboardState {
  selectedWorkspace: string | null;  // null = all workspaces
  workspaces: string[];              // derived from runs
}
```

---

## 4. Interfaces

### REST Client (lib/api.ts)

```typescript
const API_BASE = "http://127.0.0.1:7700";

async function listRuns(params?: { workspace_root?: string; status?: RunStatus }): Promise<Run[]>;
async function getRun(id: string): Promise<Run>;
async function listSteps(runId: string): Promise<Step[]>;
async function listWorktrees(workspace: string): Promise<Worktree[]>;
async function healthCheck(): Promise<{ status: string }>;

// Review workflow (requires new daemon endpoints - see §11)
async function getRunDiff(runId: string): Promise<RunDiff>;
async function scrapRun(runId: string): Promise<void>;           // delete branch
async function mergeRun(runId: string): Promise<{ commit: string }>;
async function createPR(runId: string, opts?: { title?: string; body?: string }): Promise<{ url: string }>;
```

### SSE Manager (lib/sse.ts)

```typescript
interface SSEOptions {
  onEvent: (event: RunEvent) => void;
  onOutput?: (chunk: { step_id: string; offset: number; content: string }) => void;
  onError?: (error: Error) => void;
  onReconnect?: () => void;
}

class RunEventStream {
  constructor(runId: string, options: SSEOptions);
  connect(afterTimestamp?: number): void;
  disconnect(): void;
  readonly connected: boolean;
  readonly lastEventTimestamp: number;  // for reconnection
}

class RunOutputStream {
  constructor(runId: string, options: Pick<SSEOptions, "onOutput" | "onError" | "onReconnect">);
  connect(offset?: number): void;
  disconnect(): void;
}
```

### Query Hooks (hooks/)

```typescript
// Polling for run list (5s interval when tab visible)
function useRuns(workspace?: string): UseQueryResult<Run[]>;

// Single run with SSE updates
function useRun(id: string): { run: Run | undefined; isLoading: boolean };

// Steps for a run
function useSteps(runId: string): UseQueryResult<Step[]>;

// SSE event stream
function useRunEvents(runId: string): { events: RunEvent[]; connected: boolean };

// SSE output stream
function useRunOutput(runId: string): { output: string; connected: boolean };
```

---

## 5. Workflows

### Main Flow: Run List
```
Page Load
  → GET /runs (all workspaces)
  → Extract unique workspace_root values → populate workspace switcher
  → Display run cards grouped/filtered by workspace
  → Poll every 5s for status updates (when tab visible)
```

### Drill-Down: Run Detail
```
Click Run Card
  → Navigate to /runs/{id}
  → GET /runs/{id} + GET /runs/{id}/steps
  → Connect SSE /runs/{id}/events (for status changes)
  → Connect SSE /runs/{id}/output (for log streaming)
  → Display step timeline + live log viewer
  → On disconnect: auto-reconnect with last timestamp/offset
```

### Workspace Switching
```
Select Workspace (or "All")
  → Update URL query param ?workspace=...
  → Filter displayed runs client-side (already fetched)
  → Or refetch with workspace_root filter for large deployments
```

### SSE Reconnection
```
Connection Lost
  → Exponential backoff (1s, 2s, 4s, max 30s)
  → Reconnect with ?after=lastEventTimestamp or ?offset=lastOffset
  → Dedupe events by id to handle overlap
```

### Lifecycle Checklist
For completed runs, derive checklist from events:
```
Run Completed
  → Parse events to build lifecycle steps:
     ✓ Run started
     ✓ Implementation completed (N iterations)
     ✓ Review passed
     ✓ Verification passed
     ✓ Worktree merged to branch
     ✓ Worktree cleaned up
     → Branch ready for review
```

### Review & Action Flow
```
Completed Run with changes
  → Navigate to /runs/{id}/review
  → Fetch commits and diffs from daemon
  → Choose view mode:
     [Commits] → List of commits, click to view each commit's file changes
     [All Changes] → Aggregate diff (like PR view), cycle through files
  → Toggle diff style: split (side-by-side) or unified (stacked)
  → User reviews changes
  → User selects action:
     [Scrap] → DELETE branch, update run metadata
     [Merge] → Merge run_branch into merge_target_branch
     [Create PR] → gh pr create, return PR URL
  → Show confirmation with result (PR link, merge commit, etc.)
```

### Review View Modes

**Commit View:**
```
┌─────────────────────────────────────────────┐
│ Commits (5)                                 │
├─────────────────────────────────────────────┤
│ ● abc123 - Add user authentication          │
│ ○ def456 - Fix login validation             │
│ ○ ghi789 - Add session middleware           │
│ ○ ...                                       │
├─────────────────────────────────────────────┤
│ Files changed in abc123:                    │
│   src/auth.ts (+42, -3)                     │
│   src/middleware.ts (+15, -0)               │
├─────────────────────────────────────────────┤
│ [Diff viewer for selected file]             │
└─────────────────────────────────────────────┘
```

**All Changes View (PR-style):**
```
┌─────────────────────────────────────────────┐
│ Files changed (12)         [Split|Unified]  │
├─────────────────────────────────────────────┤
│ ● src/auth.ts (+42, -3)                     │
│ ○ src/middleware.ts (+15, -0)               │
│ ○ src/routes/login.ts (+28, -5)             │
│ ○ ...                                       │
├─────────────────────────────────────────────┤
│ [Diff viewer - split or unified]            │
└─────────────────────────────────────────────┘
```

---

## 6. Error Handling

### Error Types
- **Network Error:** daemon unreachable (show banner, retry)
- **404 Run Not Found:** stale link (redirect to list with toast)
- **SSE Disconnect:** connection dropped (auto-reconnect with backoff)

### Recovery Strategy
- REST errors: show error state in component, allow retry
- SSE errors: auto-reconnect, show "reconnecting..." indicator
- Daemon down: show global banner "Daemon unavailable", poll health endpoint

---

## 7. Observability

### Logs
- Console logging for SSE connect/disconnect/reconnect events
- Console logging for query errors

### Metrics
- V0: None (future: track event lag, reconnect count)

---

## 8. Security and Privacy

### AuthZ/AuthN
- V0: No authentication (tailscale network isolation)
- Future: Support `Authorization: Bearer <token>` header from env/config

### Data Handling
- All data stays in browser memory
- No localStorage persistence of run data
- Workspace paths displayed but not editable

---

## 9. Migration or Rollout

### Compatibility Notes
- Requires loopd >= current version (with SSE endpoints)
- No breaking changes to loopd API

### Rollout Plan
1. Scaffold app with routing and basic layout
2. Implement run list with polling
3. Implement run detail with steps
4. Add SSE streaming for events and output
5. Polish: loading states, error handling, reconnection

---

## 10. Open Questions

1. **Workspace discovery** - Should dashboard auto-discover workspaces, or require explicit registration?
   - Current approach: derive from runs (no config needed, but won't show workspaces with no runs)

2. **Log viewer performance** - For very long outputs (100k+ lines), virtualized scrolling needed?
   - Start simple, add virtualization if perf issues arise

3. **Multi-run view** - Watch multiple runs simultaneously in split view?
   - Defer to V1

4. **Dark mode** - Default or toggle?
   - Start with system preference detection via Tailwind `dark:` classes

5. **Review state persistence** - Track review status (pending/reviewed/actioned) in daemon or dashboard-only?
   - Leaning toward daemon: add `review_status` field to runs table

---

## 11. Required Daemon Changes

The review workflow requires new loopd endpoints. These should be added before implementing review features in the dashboard.

### New Endpoints

#### GET /runs/{id}/diff
Returns commits and aggregate diff between base_branch and run_branch.

**Response:**
```json
{
  "base_ref": "main",
  "head_ref": "loop/run-abc123",
  "commits": [
    {
      "sha": "abc123",
      "message": "Add user authentication",
      "author": "loop-agent",
      "timestamp": "2026-02-01T10:00:00Z",
      "files": [
        { "path": "src/auth.ts", "status": "added", "patch": "...", "additions": 42, "deletions": 0 }
      ],
      "stats": { "additions": 42, "deletions": 0 }
    }
  ],
  "files": [
    { "path": "src/auth.ts", "status": "added", "patch": "...", "additions": 42, "deletions": 0 }
  ],
  "stats": { "additions": 42, "deletions": 10, "files_changed": 5 }
}
```

**Implementation:**
- `git log base_branch..run_branch --format=...` for commits
- `git diff base_branch...run_branch` for aggregate diff
- `git show <sha> --format=...` for per-commit diffs

#### POST /runs/{id}/scrap
Delete the run branch and mark run as scrapped.

**Response:** 204 No Content

**Implementation:** `git branch -D run_branch` in the workspace.

#### POST /runs/{id}/merge
Merge run_branch into merge_target_branch (or base_branch if not set).

**Request (optional):**
```json
{
  "strategy": "squash" | "merge"  // default: squash
}
```

**Response:**
```json
{
  "commit": "abc123..."
}
```

**Implementation:** `git checkout target && git merge --squash run_branch && git commit`

#### POST /runs/{id}/create-pr
Create a GitHub PR from run_branch to merge_target_branch.

**Request (optional):**
```json
{
  "title": "...",
  "body": "..."
}
```

**Response:**
```json
{
  "url": "https://github.com/owner/repo/pull/123"
}
```

**Implementation:** Shell out to `gh pr create`. Requires `gh` CLI and auth.

### New Run Fields

Add to runs table:
- `review_status`: `pending` | `reviewed` | `scrapped` | `merged` | `pr_created`
- `review_action_at`: timestamp
- `pr_url`: string (nullable)
- `merge_commit`: string (nullable)

### Daemon Spec Reference

These changes should be documented in a separate spec (e.g., `specs/daemon-review-api.md`) or added to `specs/orchestrator-daemon.md`.

---

## 12. Testing Strategy

### Overview
Tests must work without a running daemon. Use MSW to mock the API during development and testing.

### Stack
- **Unit/Component Tests:** Vitest + React Testing Library
- **API Mocking:** MSW (Mock Service Worker)
- **E2E Tests:** Playwright (headless)

### Module Layout
```
apps/dashboard/
  src/
    mocks/
      handlers.ts      # MSW request handlers
      fixtures.ts      # sample runs, steps, events
      browser.ts       # MSW browser setup
      server.ts        # MSW node setup (for tests)
  tests/
    e2e/
      run-list.spec.ts
      run-detail.spec.ts
    setup.ts           # Vitest setup with MSW
  playwright.config.ts
  vitest.config.ts
```

### MSW Handlers (src/mocks/handlers.ts)
```typescript
import { http, HttpResponse } from "msw";
import { runs, steps, events } from "./fixtures";

export const handlers = [
  http.get("http://127.0.0.1:7700/health", () => {
    return HttpResponse.json({ status: "ok" });
  }),

  http.get("http://127.0.0.1:7700/runs", () => {
    return HttpResponse.json({ runs });
  }),

  http.get("http://127.0.0.1:7700/runs/:id", ({ params }) => {
    const run = runs.find((r) => r.id === params.id);
    if (!run) return new HttpResponse(null, { status: 404 });
    return HttpResponse.json({ run });
  }),

  http.get("http://127.0.0.1:7700/runs/:id/steps", ({ params }) => {
    return HttpResponse.json({ steps: steps[params.id] ?? [] });
  }),

  // SSE endpoints need special handling - return event stream
  http.get("http://127.0.0.1:7700/runs/:id/events", ({ params }) => {
    const stream = new ReadableStream({
      start(controller) {
        const runEvents = events[params.id] ?? [];
        runEvents.forEach((event) => {
          controller.enqueue(`data: ${JSON.stringify(event)}\n\n`);
        });
      },
    });
    return new HttpResponse(stream, {
      headers: { "Content-Type": "text/event-stream" },
    });
  }),
];
```

### Agent Verification Workflow
```
Agent makes changes
  → bun test              # Vitest component tests (mocked API)
  → bun run test:e2e      # Playwright headless against MSW
  → On failure: screenshot saved to tests/e2e/screenshots/
```

### Test Commands
```bash
bun test                  # Run Vitest
bun run test:e2e          # Run Playwright
bun run test:e2e:ui       # Playwright UI mode (debugging)
```

---

## 13. Diff Viewer (@pierre/diffs)

### Package
- **npm:** `@pierre/diffs`
- **Docs:** https://diffs.com/docs
- **Repo:** https://github.com/pierrecomputer/pierre

### Installation
```bash
bun add @pierre/diffs
```

### Package Exports
| Package | Description |
|---------|-------------|
| `@pierre/diffs` | Vanilla JS components and utilities |
| `@pierre/diffs/react` | **React components** (use this) |
| `@pierre/diffs/ssr` | Server-side rendering utilities |

### Components
- **`MultiFileDiff`** - Compare two file versions (old/new contents)
- **`PatchDiff`** - Render from a patch/diff string (what we get from git)
- **`FileDiff`** - Render pre-parsed FileDiffMetadata
- **`File`** - Render single file without diff

### Usage: PatchDiff (for git diff output)
```typescript
import { PatchDiff } from "@pierre/diffs/react";

interface Props {
  patch: string;  // unified diff from `git diff`
}

export function DiffViewer({ patch }: Props) {
  return (
    <PatchDiff
      patch={patch}
      options={{
        theme: { dark: "pierre-dark", light: "pierre-light" },
        diffStyle: "split",  // or "unified"
      }}
    />
  );
}
```

### Usage: MultiFileDiff (for file contents comparison)
```typescript
import { type FileContents, MultiFileDiff } from "@pierre/diffs/react";
import { useMemo } from "react";

interface Props {
  oldContent: string;
  newContent: string;
  filename: string;
}

export function FileDiffViewer({ oldContent, newContent, filename }: Props) {
  // IMPORTANT: Keep file objects stable to avoid re-renders
  const oldFile = useMemo<FileContents>(
    () => ({ name: filename, contents: oldContent }),
    [filename, oldContent]
  );
  const newFile = useMemo<FileContents>(
    () => ({ name: filename, contents: newContent }),
    [filename, newContent]
  );

  return (
    <MultiFileDiff
      oldFile={oldFile}
      newFile={newFile}
      options={{
        theme: { dark: "pierre-dark", light: "pierre-light" },
        diffStyle: "split",
      }}
    />
  );
}
```

### Theming
Supports automatic light/dark switching:
```typescript
options={{
  theme: { dark: "pierre-dark", light: "pierre-light" }
}}
```

### Performance Note
Keep file objects stable using `useState` or `useMemo`. The component uses reference equality for change detection.

> **Nia Reference:** Full API docs indexed. Query with:
> `nia_search(query="PatchDiff options theme", data_sources=["https://diffs.com/docs"])`
