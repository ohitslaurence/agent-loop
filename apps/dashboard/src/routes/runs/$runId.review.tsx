import { createFileRoute, Link } from '@tanstack/react-router'
import { useState, useMemo } from 'react'
import { useRun } from '@/hooks/use-run'
import { useRunDiff } from '@/hooks/use-run-diff'
import { useEscapeToGoBack } from '@/hooks/use-keyboard-navigation'
import { DiffViewer } from '@/components/diff-viewer'
import { FileList } from '@/components/file-list'
import { CommitList } from '@/components/commit-list'

export const Route = createFileRoute('/runs/$runId/review')({
  component: ReviewPage,
})

type ViewMode = 'commits' | 'all'
type DiffLayout = 'split' | 'unified'

function ReviewPage() {
  const { runId } = Route.useParams()
  const { run, isLoading: runLoading, error: runError } = useRun(runId)
  const { data: diff, isLoading: diffLoading, error: diffError } = useRunDiff(runId)

  const [viewMode, setViewMode] = useState<ViewMode>('all')
  const [diffLayout, setDiffLayout] = useState<DiffLayout>('split')
  const [selectedCommit, setSelectedCommit] = useState<string | null>(null)
  const [selectedFile, setSelectedFile] = useState<string | null>(null)

  useEscapeToGoBack()

  // Get files based on view mode
  const files = useMemo(() => {
    if (!diff) return []
    if (viewMode === 'all') {
      return diff.files
    }
    // Commit view - show files for selected commit
    if (selectedCommit) {
      const commit = diff.commits.find((c) => c.sha === selectedCommit)
      return commit?.files ?? []
    }
    // Default to first commit's files
    return diff.commits[0]?.files ?? []
  }, [diff, viewMode, selectedCommit])

  // Get selected file object
  const selectedFileObj = useMemo(() => {
    if (!selectedFile) return null
    return files.find((f) => f.path === selectedFile) ?? null
  }, [files, selectedFile])

  // Auto-select first file when files change
  useMemo(() => {
    if (files.length > 0 && !files.find((f) => f.path === selectedFile)) {
      setSelectedFile(files[0].path)
    }
  }, [files, selectedFile])

  // Auto-select first commit when switching to commit view
  useMemo(() => {
    if (viewMode === 'commits' && diff?.commits.length && !selectedCommit) {
      setSelectedCommit(diff.commits[0].sha)
    }
  }, [viewMode, diff, selectedCommit])

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
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-lg font-semibold">{run.name}</h1>
            <p className="text-sm text-muted-foreground">
              {diff.commits.length} commit{diff.commits.length !== 1 ? 's' : ''} ·{' '}
              {diff.stats.files_changed} file{diff.stats.files_changed !== 1 ? 's' : ''} ·{' '}
              <span className="text-green-600">+{diff.stats.additions}</span>{' '}
              <span className="text-red-600">-{diff.stats.deletions}</span>
            </p>
          </div>
        </div>
      </div>

      {/* View controls */}
      <div className="flex items-center justify-between">
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

      {/* Main content */}
      <div className="grid grid-cols-[280px_1fr] gap-4 min-h-[600px]">
        {/* Sidebar */}
        <div className="space-y-4">
          {viewMode === 'commits' && (
            <div className="rounded-lg border border-border bg-card p-3">
              <h3 className="text-sm font-medium mb-2">
                Commits ({diff.commits.length})
              </h3>
              <CommitList
                commits={diff.commits}
                selectedSha={selectedCommit}
                onSelectCommit={setSelectedCommit}
              />
            </div>
          )}
          <div className="rounded-lg border border-border bg-card p-3">
            <h3 className="text-sm font-medium mb-2">
              Files ({files.length})
            </h3>
            <FileList
              files={files}
              selectedPath={selectedFile}
              onSelectFile={setSelectedFile}
            />
          </div>
        </div>

        {/* Diff viewer */}
        <div className="rounded-lg border border-border bg-card p-3 overflow-auto">
          <DiffViewer file={selectedFileObj} layout={diffLayout} />
        </div>
      </div>
    </div>
  )
}
