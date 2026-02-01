import { useQuery } from '@tanstack/react-query'
import { healthCheck } from '@/lib/api'

/**
 * Polls daemon health endpoint to detect connectivity issues.
 * Returns { isAvailable, isChecking, error }.
 * Polls every 5s, retries on failure with shorter interval.
 */
export function useDaemonHealth() {
  const { data, isLoading, error, isError } = useQuery({
    queryKey: ['daemon-health'],
    queryFn: healthCheck,
    refetchInterval: (query) => {
      // Poll faster when daemon is unavailable
      return query.state.status === 'error' ? 2000 : 10000
    },
    retry: 1, // Quick fail for UI feedback
    retryDelay: 1000,
  })

  return {
    isAvailable: !isError && !!data,
    isChecking: isLoading,
    error: error as Error | null,
  }
}
