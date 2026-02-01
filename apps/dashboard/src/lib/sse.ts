// SSE manager with reconnection logic (See spec §4, §5)

import type { RunEvent } from './types'

const API_BASE = 'http://127.0.0.1:7700'

// Backoff constants
const INITIAL_BACKOFF_MS = 1000
const MAX_BACKOFF_MS = 30000
const BACKOFF_MULTIPLIER = 2

export interface OutputChunk {
  step_id: string
  offset: number
  content: string
}

export interface SSEOptions {
  onEvent: (event: RunEvent) => void
  onOutput?: (chunk: OutputChunk) => void
  onError?: (error: Error) => void
  onReconnect?: () => void
}

/**
 * SSE stream for run events (/runs/{id}/events)
 * Handles reconnection with exponential backoff and event deduplication.
 */
export class RunEventStream {
  private readonly runId: string
  private readonly options: SSEOptions
  private eventSource: EventSource | null = null
  private backoff = INITIAL_BACKOFF_MS
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null
  private seenEventIds = new Set<string>()
  private _connected = false
  private _lastEventTimestamp = 0

  constructor(runId: string, options: SSEOptions) {
    this.runId = runId
    this.options = options
  }

  get connected(): boolean {
    return this._connected
  }

  get lastEventTimestamp(): number {
    return this._lastEventTimestamp
  }

  connect(afterTimestamp?: number): void {
    if (this.eventSource) {
      return // Already connected
    }

    const url = new URL(`${API_BASE}/runs/${this.runId}/events`)
    if (afterTimestamp !== undefined && afterTimestamp > 0) {
      url.searchParams.set('after', afterTimestamp.toString())
    }

    this.eventSource = new EventSource(url.toString())

    this.eventSource.onopen = () => {
      this._connected = true
      this.backoff = INITIAL_BACKOFF_MS // Reset backoff on successful connection
    }

    this.eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data) as RunEvent
        // Dedupe events by id to handle overlap during reconnection
        if (this.seenEventIds.has(data.id)) {
          return
        }
        this.seenEventIds.add(data.id)
        // Track timestamp for reconnection
        if (data.timestamp > this._lastEventTimestamp) {
          this._lastEventTimestamp = data.timestamp
        }
        this.options.onEvent(data)
      } catch (err) {
        this.options.onError?.(
          err instanceof Error ? err : new Error('Failed to parse event')
        )
      }
    }

    this.eventSource.onerror = () => {
      this._connected = false
      this.eventSource?.close()
      this.eventSource = null
      this.scheduleReconnect()
    }
  }

  disconnect(): void {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }
    if (this.eventSource) {
      this.eventSource.close()
      this.eventSource = null
    }
    this._connected = false
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimeout) {
      return // Already scheduled
    }

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      this.options.onReconnect?.()
      this.connect(this._lastEventTimestamp)
      // Increase backoff for next potential failure
      this.backoff = Math.min(this.backoff * BACKOFF_MULTIPLIER, MAX_BACKOFF_MS)
    }, this.backoff)
  }
}

/**
 * SSE stream for run output (/runs/{id}/output)
 * Handles reconnection with exponential backoff.
 */
export class RunOutputStream {
  private readonly runId: string
  private readonly options: Pick<SSEOptions, 'onOutput' | 'onError' | 'onReconnect'>
  private eventSource: EventSource | null = null
  private backoff = INITIAL_BACKOFF_MS
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null
  private _connected = false
  private _lastOffset = 0

  constructor(
    runId: string,
    options: Pick<SSEOptions, 'onOutput' | 'onError' | 'onReconnect'>
  ) {
    this.runId = runId
    this.options = options
  }

  get connected(): boolean {
    return this._connected
  }

  get lastOffset(): number {
    return this._lastOffset
  }

  connect(offset?: number): void {
    if (this.eventSource) {
      return // Already connected
    }

    const url = new URL(`${API_BASE}/runs/${this.runId}/output`)
    if (offset !== undefined && offset > 0) {
      url.searchParams.set('offset', offset.toString())
    }

    this.eventSource = new EventSource(url.toString())

    this.eventSource.onopen = () => {
      this._connected = true
      this.backoff = INITIAL_BACKOFF_MS // Reset backoff on successful connection
    }

    this.eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data) as OutputChunk
        // Track offset for reconnection
        const endOffset = data.offset + data.content.length
        if (endOffset > this._lastOffset) {
          this._lastOffset = endOffset
        }
        this.options.onOutput?.(data)
      } catch (err) {
        this.options.onError?.(
          err instanceof Error ? err : new Error('Failed to parse output chunk')
        )
      }
    }

    this.eventSource.onerror = () => {
      this._connected = false
      this.eventSource?.close()
      this.eventSource = null
      this.scheduleReconnect()
    }
  }

  disconnect(): void {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }
    if (this.eventSource) {
      this.eventSource.close()
      this.eventSource = null
    }
    this._connected = false
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimeout) {
      return // Already scheduled
    }

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      this.options.onReconnect?.()
      this.connect(this._lastOffset)
      // Increase backoff for next potential failure
      this.backoff = Math.min(this.backoff * BACKOFF_MULTIPLIER, MAX_BACKOFF_MS)
    }, this.backoff)
  }
}
