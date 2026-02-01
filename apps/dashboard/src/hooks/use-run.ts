import { useQuery } from '@tanstack/react-query'
import { getRun } from '@/lib/api'
import type { Run } from '@/lib/types'

/**
 * Fetches a single run by ID.
 * See spec §4: useRun(id: string): { run: Run | undefined; isLoading: boolean }
 */
export function useRun(id: string): { run: Run | undefined; isLoading: boolean; error: Error | null } {
  const { data, isLoading, error } = useQuery({
    queryKey: ['run', id],
    queryFn: () => getRun(id),
  })

  return { run: data, isLoading, error }
}
