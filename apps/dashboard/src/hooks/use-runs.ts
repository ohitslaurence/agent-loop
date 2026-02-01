import { useQuery } from '@tanstack/react-query'
import { listRuns } from '@/lib/api'

/**
 * Fetches runs list with 5s polling when tab is visible.
 * See spec §4: useRuns(workspace?: string): UseQueryResult<Run[]>
 */
export function useRuns(workspace?: string) {
  return useQuery({
    queryKey: ['runs', workspace ?? 'all'],
    queryFn: () => listRuns(workspace ? { workspace_root: workspace } : undefined),
    refetchInterval: 5000, // 5s polling per spec §5
    refetchIntervalInBackground: false, // Only poll when tab visible
  })
}
