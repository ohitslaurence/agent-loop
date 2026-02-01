import { useQuery } from '@tanstack/react-query'
import { getRunDiff } from '@/lib/api'
import type { RunDiff } from '@/lib/types'

export function useRunDiff(runId: string) {
  return useQuery<RunDiff>({
    queryKey: ['run-diff', runId],
    queryFn: () => getRunDiff(runId),
    staleTime: 30_000, // Cache for 30s since diffs don't change often
  })
}
