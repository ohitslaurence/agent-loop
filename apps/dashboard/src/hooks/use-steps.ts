import { useQuery, type UseQueryResult } from '@tanstack/react-query'
import { listSteps } from '@/lib/api'
import type { Step } from '@/lib/types'

/**
 * Fetches steps for a run.
 * See spec §4: useSteps(runId: string): UseQueryResult<Step[]>
 */
export function useSteps(runId: string): UseQueryResult<Step[]> {
  return useQuery({
    queryKey: ['steps', runId],
    queryFn: () => listSteps(runId),
  })
}
