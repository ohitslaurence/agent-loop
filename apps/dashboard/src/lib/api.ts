// REST client for loopd daemon (See spec §4)

import type {
  Run,
  RunStatus,
  Step,
  StepPhase,
  StepStatus,
  RunDiff,
  ReviewStatus,
  RunWorktree,
} from './types'

const API_BASE = '/api'

// Normalize phase from API (lowercase) to expected format (PascalCase)
function normalizePhase(phase: string): StepPhase {
  const map: Record<string, StepPhase> = {
    implementation: 'Implementation',
    review: 'Review',
    verification: 'Verification',
    watchdog: 'Watchdog',
    merge: 'Merge',
  }
  return map[phase.toLowerCase()] ?? ('Implementation' as StepPhase)
}

// Normalize status from API (UPPERCASE) to expected format (PascalCase)
function normalizeStepStatus(status: string): StepStatus {
  const map: Record<string, StepStatus> = {
    pending: 'Pending',
    queued: 'Pending',
    in_progress: 'Running',
    running: 'Running',
    retrying: 'Running',
    succeeded: 'Succeeded',
    failed: 'Failed',
    canceled: 'Failed',
    cancelled: 'Failed',
  }
  return map[status.toLowerCase()] ?? ('Pending' as StepStatus)
}

function normalizeRunStatus(status: string): RunStatus {
  const map: Record<string, RunStatus> = {
    pending: 'Pending',
    running: 'Running',
    completed: 'Completed',
    failed: 'Failed',
    canceled: 'Canceled',
    cancelled: 'Canceled',
    paused: 'Paused',
  }
  return map[status.toLowerCase()] ?? ('Pending' as RunStatus)
}

function normalizeReviewStatus(status?: string): ReviewStatus | undefined {
  if (!status) return undefined
  const map: Record<string, ReviewStatus> = {
    pending: 'Pending',
    reviewed: 'Reviewed',
    scrapped: 'Scrapped',
    merged: 'Merged',
    pr_created: 'PrCreated',
  }
  return map[status.toLowerCase()]
}

function normalizeMergeStrategy(strategy?: string): RunWorktree['merge_strategy'] {
  if (!strategy) return 'None'
  const map: Record<string, RunWorktree['merge_strategy']> = {
    none: 'None',
    squash: 'Squash',
    merge: 'MergeLast',
    mergelast: 'MergeLast',
    merge_last: 'MergeLast',
  }
  return map[strategy.toLowerCase()] ?? 'None'
}

function normalizeProvider(provider?: string): RunWorktree['provider'] {
  if (!provider) return 'Native'
  const map: Record<string, RunWorktree['provider']> = {
    git: 'Native',
    native: 'Native',
    auto: 'Native',
    worktrunk: 'Worktrunk',
  }
  return map[provider.toLowerCase()] ?? 'Native'
}

function normalizeRun(raw: Run): Run {
  return {
    ...raw,
    status: normalizeRunStatus(raw.status as string),
    review_status: normalizeReviewStatus(raw.review_status as string | undefined),
    worktree: raw.worktree
      ? {
          ...raw.worktree,
          merge_strategy: normalizeMergeStrategy(raw.worktree.merge_strategy as string),
          provider: normalizeProvider(raw.worktree.provider as string),
        }
      : undefined,
  }
}

// Normalize step from API response
function normalizeStep(raw: Record<string, unknown>): Step {
  return {
    id: raw.id as string,
    run_id: raw.run_id as string,
    phase: normalizePhase(raw.phase as string),
    status: normalizeStepStatus(raw.status as string),
    attempt: raw.attempt as number,
    started_at: raw.started_at as string | undefined,
    completed_at: (raw.completed_at ?? raw.ended_at) as string | undefined,
    output_path: raw.output_path as string | undefined,
    exit_code: raw.exit_code as number | undefined,
  }
}

// Health check
export async function healthCheck(): Promise<{ status: string }> {
  const res = await fetch(`${API_BASE}/health`)
  if (!res.ok) {
    throw new Error(`Health check failed: ${res.status}`)
  }
  return res.json()
}

// List runs with optional filters
export async function listRuns(params?: {
  workspace_root?: string
  status?: RunStatus
}): Promise<Run[]> {
  const searchParams = new URLSearchParams()
  if (params?.workspace_root) {
    searchParams.set('workspace_root', params.workspace_root)
  }
  if (params?.status) {
    searchParams.set('status', params.status)
  }
  const query = searchParams.toString()
  const url = `${API_BASE}/runs${query ? `?${query}` : ''}`
  const res = await fetch(url)
  if (!res.ok) {
    throw new Error(`Failed to list runs: ${res.status}`)
  }
  const data = await res.json()
  return (data.runs as Run[]).map(normalizeRun)
}

// Get a single run by ID
export async function getRun(id: string): Promise<Run> {
  const res = await fetch(`${API_BASE}/runs/${id}`)
  if (!res.ok) {
    if (res.status === 404) {
      throw new Error(`Run not found: ${id}`)
    }
    throw new Error(`Failed to get run: ${res.status}`)
  }
  const data = await res.json()
  return normalizeRun(data.run as Run)
}

// List steps for a run
export async function listSteps(runId: string): Promise<Step[]> {
  const res = await fetch(`${API_BASE}/runs/${runId}/steps`)
  if (!res.ok) {
    throw new Error(`Failed to list steps: ${res.status}`)
  }
  const data = await res.json()
  return (data.steps as Record<string, unknown>[]).map(normalizeStep)
}

// Worktree types (not in main types.ts as they're specific to this endpoint)
export interface Worktree {
  path: string
  branch: string
  head_sha: string
}

// List worktrees for a workspace
export async function listWorktrees(workspace: string): Promise<Worktree[]> {
  const searchParams = new URLSearchParams({ workspace })
  const res = await fetch(`${API_BASE}/worktrees?${searchParams}`)
  if (!res.ok) {
    throw new Error(`Failed to list worktrees: ${res.status}`)
  }
  const data = await res.json()
  return data.worktrees
}

// Review workflow endpoints (requires daemon changes per spec §11)

// Get diff for a run (commits + aggregate diff)
export async function getRunDiff(runId: string): Promise<RunDiff> {
  const res = await fetch(`${API_BASE}/runs/${runId}/diff`)
  if (!res.ok) {
    throw new Error(`Failed to get run diff: ${res.status}`)
  }
  return res.json()
}

// Scrap a run (delete branch)
export async function scrapRun(runId: string): Promise<void> {
  const res = await fetch(`${API_BASE}/runs/${runId}/scrap`, {
    method: 'POST',
  })
  if (!res.ok) {
    throw new Error(`Failed to scrap run: ${res.status}`)
  }
}

// Merge a run
export async function mergeRun(
  runId: string
): Promise<{ commit: string }> {
  const res = await fetch(`${API_BASE}/runs/${runId}/merge`, {
    method: 'POST',
  })
  if (!res.ok) {
    throw new Error(`Failed to merge run: ${res.status}`)
  }
  return res.json()
}

// Create PR for a run
export async function createPR(
  runId: string,
  opts?: { title?: string; body?: string }
): Promise<{ url: string }> {
  const res = await fetch(`${API_BASE}/runs/${runId}/create-pr`, {
    method: 'POST',
    headers: opts ? { 'Content-Type': 'application/json' } : undefined,
    body: opts ? JSON.stringify(opts) : undefined,
  })
  if (!res.ok) {
    throw new Error(`Failed to create PR: ${res.status}`)
  }
  return res.json()
}
