import { createFileRoute, Link } from '@tanstack/react-router'
import { useState, useCallback } from 'react'
import { useRun } from '@/hooks/use-run'
import { useRunDiff } from '@/hooks/use-run-diff'
import { useEscapeToGoBack } from '@/hooks/use-keyboard-navigation'
import { DiffViewer } from '@/components/diff-viewer'
import { ReviewActions } from '@/components/review-actions'

export const Route = createFileRoute('/runs/$runId/review')({
  component: ReviewPage,
})

type ViewMode = 'commits' | 'all'
type DiffLayout = 'split' | 'unified'

function ReviewPage() {
  const { runId } = Route.useParams()
  const { run, isLoading: runLoading, error: runError } = useRun(runId)
  const { data: diff, isLoading: diffLoading, error: diffError } = useRunDiff(runId, run?.status)

  const [viewMode, setViewMode] = useState<ViewMode>('all')
  const [diffLayout, setDiffLayout] = useState<DiffLayout>('unified')
  const [collapsedFiles, setCollapsedFiles] = useState<Set<string>>(new Set())
  const [collapsedCommits, setCollapsedCommits] = useState<Set<string>>(new Set())
  const isEmptyDiff = diff?.commits.length === 0 && diff.files.length === 0

  useEscapeToGoBack()

  const toggleFile = useCallback((key: string) => {
    setCollapsedFiles((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }, [])

  const toggleCommit = useCallback((sha: string) => {
    setCollapsedCommits((prev) => {
      const next = new Set(prev)
      if (next.has(sha)) next.delete(sha)
      else next.add(sha)
      return next
    })
  }, [])

  const collapseAll = useCallback(() => {
    if (!diff) return
    if (viewMode === 'all') {
      setCollapsedFiles(new Set(diff.files.map((f) => f.path)))
    } else {
      setCollapsedCommits(new Set(diff.commits.map((c) => c.sha)))
      const allFileKeys = diff.commits.flatMap((c) => c.files.map((f) => `${c.sha}:${f.path}`))
      setCollapsedFiles(new Set(allFileKeys))
    }
  }, [diff, viewMode])

  const expandAll = useCallback(() => {
    setCollapsedFiles(new Set())
    setCollapsedCommits(new Set())
  }, [])

  if (runLoading || diffLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-muted-foreground">Loading diff...</div>
      </div>
    )
  }

  const error = runError || diffError
  if (error) {
    return (
      <div className="space-y-4">
        <Link
          to="/runs/$runId"
          params={{ runId }}
          className="text-sm text-muted-foreground hover:underline"
        >
          &larr; Back to run
        </Link>
        <div className="rounded-md border border-destructive/50 bg-destructive/10 p-4">
          <p className="text-destructive">Failed to load diff: {error.message}</p>
        </div>
      </div>
    )
  }

  if (!run || !diff) {
    return (
      <div className="space-y-4">
        <Link
          to="/runs/$runId"
          params={{ runId }}
          className="text-sm text-muted-foreground hover:underline"
        >
          &larr; Back to run
        </Link>
        <div className="py-12 text-center">
          <p className="text-muted-foreground">No diff available</p>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <Link
          to="/runs/$runId"
          params={{ runId }}
          className="text-sm text-muted-foreground hover:underline"
        >
          &larr; Back to run
        </Link>
        <div className="text-sm text-muted-foreground">
          {diff.base_ref} → {diff.head_ref}
        </div>
      </div>

      {/* Run info */}
      <div className="rounded-lg border border-border bg-card p-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
          <div className="min-w-0 flex-1">
            <h1 className="text-lg font-semibold truncate">{run.name}</h1>
            <p className="text-sm text-muted-foreground">
              {diff.commits.length} commit{diff.commits.length !== 1 ? 's' : ''} ·{' '}
              {diff.stats.files_changed} file{diff.stats.files_changed !== 1 ? 's' : ''} ·{' '}
              <span className="text-green-600">+{diff.stats.additions}</span>{' '}
              <span className="text-red-600">-{diff.stats.deletions}</span>
            </p>
          </div>
          {(run.status === 'Completed' || run.status === 'Paused') && <ReviewActions run={run} />}
        </div>
      </div>

      {isEmptyDiff && (
        <div className="rounded-lg border border-border bg-muted/40 p-3 text-sm text-muted-foreground">
          <p>
            No commits or file changes found between {diff.base_ref} and {diff.head_ref}.
          </p>
          {run.worktree_cleanup_status === 'cleaned' && (
            <p className="mt-1">
              Worktree cleanup already ran, so uncommitted changes were removed.
            </p>
          )}
        </div>
      )}

      {/* View controls */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setViewMode('commits')}
            className={`px-3 py-1.5 text-sm rounded transition-colors ${
              viewMode === 'commits'
                ? 'bg-primary text-primary-foreground'
                : 'bg-muted hover:bg-muted/80'
            }`}
          >
            Commits
          </button>
          <button
            onClick={() => setViewMode('all')}
            className={`px-3 py-1.5 text-sm rounded transition-colors ${
              viewMode === 'all'
                ? 'bg-primary text-primary-foreground'
                : 'bg-muted hover:bg-muted/80'
            }`}
          >
            All Changes
          </button>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={collapseAll}
            className="px-3 py-1.5 text-sm rounded bg-muted hover:bg-muted/80 transition-colors"
          >
            Collapse All
          </button>
          <button
            onClick={expandAll}
            className="px-3 py-1.5 text-sm rounded bg-muted hover:bg-muted/80 transition-colors"
          >
            Expand All
          </button>
          <div className="hidden sm:flex items-center gap-2 ml-2 pl-2 border-l border-border">
            <button
              onClick={() => setDiffLayout('split')}
              className={`px-3 py-1.5 text-sm rounded transition-colors ${
                diffLayout === 'split'
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-muted hover:bg-muted/80'
              }`}
            >
              Split
            </button>
            <button
              onClick={() => setDiffLayout('unified')}
              className={`px-3 py-1.5 text-sm rounded transition-colors ${
                diffLayout === 'unified'
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-muted hover:bg-muted/80'
              }`}
            >
              Unified
            </button>
          </div>
        </div>
      </div>

      {/* File diffs */}
      <div className="space-y-3">
        {viewMode === 'all' ? (
          diff.files.map((file) => (
            <DiffViewer
              key={file.path}
              file={file}
              layout={diffLayout}
              collapsed={collapsedFiles.has(file.path)}
              onToggleCollapse={() => toggleFile(file.path)}
            />
          ))
        ) : (
          diff.commits.map((commit) => {
            const commitCollapsed = collapsedCommits.has(commit.sha)
            return (
              <div key={commit.sha} className="space-y-2">
                <div
                  className="rounded-lg border border-border bg-card px-4 py-3 cursor-pointer hover:bg-muted/50 select-none transition-colors"
                  onClick={() => toggleCommit(commit.sha)}
                >
                  <div className="flex items-start gap-2">
                    <span className="text-muted-foreground text-xs pt-1 shrink-0">
                      {commitCollapsed ? '▶' : '▼'}
                    </span>
                    <span className="text-xs font-mono text-muted-foreground shrink-0 pt-0.5">
                      {commit.sha.slice(0, 7)}
                    </span>
                    <div className="flex-1 min-w-0">
                      <div className="text-sm truncate">{commit.message}</div>
                      <div className="text-xs text-muted-foreground flex items-center gap-2">
                        <span>{commit.author}</span>
                        <span>·</span>
                        <span>{commit.files.length} file{commit.files.length !== 1 ? 's' : ''}</span>
                        <span>·</span>
                        <span className="text-green-600">+{commit.stats.additions}</span>
                        <span className="text-red-600">-{commit.stats.deletions}</span>
                      </div>
                    </div>
                  </div>
                </div>
                {!commitCollapsed && (
                  <div className="space-y-2 pl-4">
                    {commit.files.map((file) => {
                      const fileKey = `${commit.sha}:${file.path}`
                      return (
                        <DiffViewer
                          key={fileKey}
                          file={file}
                          layout={diffLayout}
                          collapsed={collapsedFiles.has(fileKey)}
                          onToggleCollapse={() => toggleFile(fileKey)}
                        />
                      )
                    })}
                  </div>
                )}
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}
