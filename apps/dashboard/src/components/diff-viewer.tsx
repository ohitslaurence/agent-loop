import { PatchDiff } from '@pierre/diffs/react'

export interface DiffFileData {
  path: string
  old_path?: string
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  additions: number
  deletions: number
  patch?: string
}

interface DiffViewerProps {
  file: DiffFileData | null
  layout: 'split' | 'unified'
  collapsed?: boolean
  onToggleCollapse?: () => void
}

export function DiffViewer({ file, layout, collapsed, onToggleCollapse }: DiffViewerProps) {
  if (!file) {
    return (
      <div className="flex items-center justify-center h-64 text-muted-foreground">
        Select a file to view diff
      </div>
    )
  }

  const isCollapsible = onToggleCollapse !== undefined
  const isCollapsed = collapsed ?? false

  return (
    <div className="rounded border border-border overflow-hidden">
      <div
        className={`bg-muted px-3 py-2 border-b border-border flex items-center justify-between ${isCollapsible ? 'cursor-pointer hover:bg-muted/80 select-none' : ''}`}
        onClick={onToggleCollapse}
      >
        <div className="flex items-center gap-2 font-mono text-sm min-w-0">
          {isCollapsible && (
            <span className="text-muted-foreground text-xs shrink-0">
              {isCollapsed ? '▶' : '▼'}
            </span>
          )}
          <FileStatusBadge status={file.status} />
          <span className="truncate">{file.path}</span>
          {file.old_path && (
            <span className="text-muted-foreground truncate">← {file.old_path}</span>
          )}
        </div>
        <div className="text-xs text-muted-foreground shrink-0 ml-2">
          <span className="text-green-600">+{file.additions}</span>
          {' / '}
          <span className="text-red-600">-{file.deletions}</span>
        </div>
      </div>
      {!isCollapsed && file.patch && (
        <div className="diff-container">
          <PatchDiff patch={file.patch} options={{ diffStyle: layout }} />
        </div>
      )}
      {!isCollapsed && !file.patch && (
        <div className="flex items-center justify-center h-16 text-muted-foreground text-sm">
          No changes in this file
        </div>
      )}
    </div>
  )
}

function FileStatusBadge({ status }: { status: 'added' | 'modified' | 'deleted' | 'renamed' }) {
  const colors = {
    added: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    modified: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200',
    deleted: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
    renamed: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200',
  }
  const labels = {
    added: 'Added',
    modified: 'Modified',
    deleted: 'Deleted',
    renamed: 'Renamed',
  }
  return (
    <span className={`px-1.5 py-0.5 rounded text-xs font-medium ${colors[status]}`}>
      {labels[status]}
    </span>
  )
}
