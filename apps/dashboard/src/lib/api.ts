// REST client for loopd daemon (See spec §4)

import type { Run, RunStatus, Step, RunDiff } from './types'

const API_BASE = 'http://127.0.0.1:7700'

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
  const url = new URL(`${API_BASE}/runs`)
  if (params?.workspace_root) {
    url.searchParams.set('workspace_root', params.workspace_root)
  }
  if (params?.status) {
    url.searchParams.set('status', params.status)
  }
  const res = await fetch(url.toString())
  if (!res.ok) {
    throw new Error(`Failed to list runs: ${res.status}`)
  }
  const data = await res.json()
  return data.runs
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
  return data.run
}

// List steps for a run
export async function listSteps(runId: string): Promise<Step[]> {
  const res = await fetch(`${API_BASE}/runs/${runId}/steps`)
  if (!res.ok) {
    throw new Error(`Failed to list steps: ${res.status}`)
  }
  const data = await res.json()
  return data.steps
}

// Worktree types (not in main types.ts as they're specific to this endpoint)
export interface Worktree {
  path: string
  branch: string
  head_sha: string
}

// List worktrees for a workspace
export async function listWorktrees(workspace: string): Promise<Worktree[]> {
  const url = new URL(`${API_BASE}/worktrees`)
  url.searchParams.set('workspace', workspace)
  const res = await fetch(url.toString())
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
