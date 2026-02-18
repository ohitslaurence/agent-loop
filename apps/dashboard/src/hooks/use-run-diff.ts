import { useQuery } from '@tanstack/react-query'
import { getRunDiff } from '@/lib/api'
import type { RunDiff } from '@/lib/types'

export function useRunDiff(runId: string, runStatus?: string) {
  const isRunning = runStatus === 'Running'
  return useQuery<RunDiff>({
    queryKey: ['run-diff', runId],
    queryFn: () => getRunDiff(runId),
    staleTime: isRunning ? 5_000 : 30_000,
    refetchInterval: isRunning ? 10_000 : false,
  })
}
