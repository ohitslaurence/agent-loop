import { AlertCircle, RefreshCw } from 'lucide-react'
import { useDaemonHealth } from '@/hooks/use-daemon-health'
import { useQueryClient } from '@tanstack/react-query'

/**
 * Global banner shown when daemon is unavailable.
 * See spec §6: "Daemon down: show global banner 'Daemon unavailable', poll health endpoint"
 */
export function DaemonStatusBanner() {
  const { isAvailable, isChecking, error } = useDaemonHealth()
  const queryClient = useQueryClient()

  // Don't show banner if daemon is available or still checking
  if (isAvailable || isChecking) {
    return null
  }

  const handleRetry = () => {
    queryClient.invalidateQueries({ queryKey: ['daemon-health'] })
  }

  return (
    <div className="border-b border-destructive/50 bg-destructive/10 px-4 py-2">
      <div className="container mx-auto flex items-center justify-between gap-4">
        <div className="flex items-center gap-2 text-sm text-destructive">
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>
            Daemon unavailable
            {error?.message && (
              <span className="text-destructive/70"> - {error.message}</span>
            )}
          </span>
        </div>
        <button
          onClick={handleRetry}
          className="flex items-center gap-1 text-sm text-destructive hover:underline"
        >
          <RefreshCw className="h-3 w-3" />
          Retry
        </button>
      </div>
    </div>
  )
}
