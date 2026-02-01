import { cn } from '@/lib/utils'
import type { RunCommit } from '@/lib/types'

interface CommitListProps {
  commits: RunCommit[]
  selectedSha: string | null
  onSelectCommit: (sha: string) => void
}

export function CommitList({ commits, selectedSha, onSelectCommit }: CommitListProps) {
  return (
    <div className="space-y-1">
      {commits.map((commit) => (
        <button
          key={commit.sha}
          onClick={() => onSelectCommit(commit.sha)}
          className={cn(
            'w-full text-left px-2 py-2 rounded transition-colors',
            selectedSha === commit.sha
              ? 'bg-accent text-accent-foreground'
              : 'hover:bg-muted'
          )}
        >
          <div className="flex items-start gap-2">
            <span className="text-xs font-mono text-muted-foreground shrink-0 pt-0.5">
              {commit.sha.slice(0, 7)}
            </span>
            <div className="flex-1 min-w-0">
              <div className="text-sm truncate">{commit.message}</div>
              <div className="text-xs text-muted-foreground flex items-center gap-2">
                <span>{commit.author}</span>
                <span>·</span>
                <span>{formatTimestamp(commit.timestamp)}</span>
                <span>·</span>
                <span className="text-green-600">+{commit.stats.additions}</span>
                <span className="text-red-600">-{commit.stats.deletions}</span>
              </div>
            </div>
          </div>
        </button>
      ))}
    </div>
  )
}

function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp)
  const now = new Date()
  const diffMs = now.getTime() - date.getTime()
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60))

  if (diffHours < 1) {
    const diffMins = Math.floor(diffMs / (1000 * 60))
    return `${diffMins}m ago`
  }
  if (diffHours < 24) {
    return `${diffHours}h ago`
  }
  const diffDays = Math.floor(diffHours / 24)
  if (diffDays < 7) {
    return `${diffDays}d ago`
  }
  return date.toLocaleDateString()
}
