import { useCallback, useEffect, useRef, useState } from 'react'
import { RunOutputStream, type OutputChunk } from '@/lib/sse'

/**
 * SSE output stream hook for run logs.
 * See spec §4: useRunOutput(runId: string): { output: string; connected: boolean }
 */
export function useRunOutput(runId: string): {
  output: string
  connected: boolean
} {
  const [output, setOutput] = useState('')
  const [connected, setConnected] = useState(false)
  const streamRef = useRef<RunOutputStream | null>(null)

  const handleOutput = useCallback((chunk: OutputChunk) => {
    setOutput((prev) => prev + chunk.content)
  }, [])

  const handleError = useCallback((error: Error) => {
    console.error('[SSE] Output stream error:', error.message)
  }, [])

  const handleReconnect = useCallback(() => {
    console.log('[SSE] Reconnecting output stream...')
  }, [])

  useEffect(() => {
    // Reset state when runId changes
    setOutput('')
    setConnected(false)

    const stream = new RunOutputStream(runId, {
      onOutput: handleOutput,
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
  }, [runId, handleOutput, handleError, handleReconnect])

  return { output, connected }
}
