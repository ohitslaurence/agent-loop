import { cn } from '@/lib/utils'
import type { DiffFile } from '@/lib/types'

interface FileListProps {
  files: DiffFile[]
  selectedPath: string | null
  onSelectFile: (path: string) => void
}

export function FileList({ files, selectedPath, onSelectFile }: FileListProps) {
  return (
    <div className="space-y-1">
      {files.map((file) => (
        <button
          key={file.path}
          onClick={() => onSelectFile(file.path)}
          className={cn(
            'w-full text-left px-2 py-1.5 rounded text-sm font-mono truncate transition-colors',
            selectedPath === file.path
              ? 'bg-accent text-accent-foreground'
              : 'hover:bg-muted'
          )}
        >
          <div className="flex items-center gap-2">
            <FileStatusIcon status={file.status} />
            <span className="truncate flex-1">{file.path}</span>
            <span className="text-xs text-muted-foreground shrink-0">
              <span className="text-green-600">+{file.additions}</span>
              {' '}
              <span className="text-red-600">-{file.deletions}</span>
            </span>
          </div>
        </button>
      ))}
    </div>
  )
}

function FileStatusIcon({ status }: { status: DiffFile['status'] }) {
  const colors = {
    added: 'text-green-600',
    modified: 'text-yellow-600',
    deleted: 'text-red-600',
    renamed: 'text-blue-600',
  }
  const labels = {
    added: 'A',
    modified: 'M',
    deleted: 'D',
    renamed: 'R',
  }
  return (
    <span className={cn('text-xs font-bold w-4 shrink-0', colors[status])}>
      {labels[status]}
    </span>
  )
}
