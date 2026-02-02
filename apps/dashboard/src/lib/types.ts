// Core types mirrored from loopd (See spec §3)

export type RunStatus =
  | 'Pending'
  | 'Running'
  | 'Completed'
  | 'Failed'
  | 'Canceled'
  | 'Paused'

export type StepPhase =
  | 'Implementation'
  | 'Review'
  | 'Verification'
  | 'Watchdog'
  | 'Merge'

export type StepStatus = 'Pending' | 'Running' | 'Succeeded' | 'Failed'

export type ReviewStatus = 'Pending' | 'Reviewed' | 'Scrapped' | 'Merged' | 'PrCreated'

export interface RunWorktree {
  worktree_path: string
  run_branch: string
  base_branch: string
  merge_target_branch?: string
  merge_strategy: 'None' | 'Squash' | 'MergeLast'
  provider: 'Native' | 'Worktrunk'
}

export interface Run {
  id: string
  name: string
  name_source: string
  status: RunStatus
  workspace_root: string
  spec_path: string
  plan_path?: string
  worktree?: RunWorktree
  worktree_cleanup_status?: 'cleaned' | 'failed' | 'skipped' | 'deferred'
  created_at: string // ISO 8601
  updated_at: string
  // Review workflow fields
  review_status?: ReviewStatus
  pr_url?: string
  merge_commit?: string
}

export interface Step {
  id: string
  run_id: string
  phase: StepPhase
  status: StepStatus
  attempt: number
  started_at?: string
  completed_at?: string
  output_path?: string
  exit_code?: number
}

export interface RunEvent {
  id: string
  run_id: string
  step_id?: string
  event_type: string
  timestamp: number // ms since epoch
  payload: Record<string, unknown>
}

// Review workflow types

export interface LifecycleStep {
  label: string
  completed: boolean
  inProgress?: boolean
  timestamp?: string
}

export interface DiffFile {
  path: string
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  old_path?: string
  patch: string
  additions: number
  deletions: number
}

export interface RunCommit {
  sha: string
  message: string
  author: string
  timestamp: string
  files: DiffFile[]
  stats: { additions: number; deletions: number }
}

export interface RunDiff {
  base_ref: string
  head_ref: string
  commits: RunCommit[]
  files: DiffFile[]
  stats: { additions: number; deletions: number; files_changed: number }
}

export type ReviewAction = 'scrapped' | 'merged' | 'pr_created'

export interface ReviewResult {
  action: ReviewAction
  pr_url?: string
  merge_commit?: string
}

// Client state

export interface DashboardState {
  selectedWorkspace: string | null
  workspaces: string[]
}
