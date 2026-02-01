import { useCallback, useEffect, useRef, useState } from 'react'
import { RunEventStream } from '@/lib/sse'
import type { RunEvent } from '@/lib/types'

/**
 * SSE event stream hook for run events.
 * See spec §4: useRunEvents(runId: string): { events: RunEvent[]; connected: boolean }
 */
export function useRunEvents(runId: string): {
  events: RunEvent[]
  connected: boolean
} {
  const [events, setEvents] = useState<RunEvent[]>([])
  const [connected, setConnected] = useState(false)
  const streamRef = useRef<RunEventStream | null>(null)

  const handleEvent = useCallback((event: RunEvent) => {
    setEvents((prev) => [...prev, event])
  }, [])

  const handleError = useCallback((error: Error) => {
    console.error('[SSE] Event stream error:', error.message)
  }, [])

  const handleReconnect = useCallback(() => {
    console.log('[SSE] Reconnecting event stream...')
  }, [])

  useEffect(() => {
    // Reset state when runId changes
    setEvents([])
    setConnected(false)

    const stream = new RunEventStream(runId, {
      onEvent: handleEvent,
      onError: handleError,
      onReconnect: handleReconnect,
    })

    streamRef.current = stream
    stream.connect()

    // Poll connection status
    const statusInterval = setInterval(() => {
      setConnected(stream.connected)
    }, 100)

    return () => {
      clearInterval(statusInterval)
      stream.disconnect()
      streamRef.current = null
    }
  }, [runId, handleEvent, handleError, handleReconnect])

  return { events, connected }
}
