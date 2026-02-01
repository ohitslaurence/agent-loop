import { http, HttpResponse } from 'msw'
import { runs, steps, events } from './fixtures'

const API_BASE = 'http://127.0.0.1:7700'

export const handlers = [
  // Health check
  http.get(`${API_BASE}/health`, () => {
    return HttpResponse.json({ status: 'ok' })
  }),

  // List all runs
  http.get(`${API_BASE}/runs`, () => {
    return HttpResponse.json({ runs })
  }),

  // Get steps for a run (must come before :id to avoid matching /runs/:id/steps as :id)
  http.get(`${API_BASE}/runs/:id/steps`, ({ params }) => {
    const id = params.id as string
    return HttpResponse.json({ steps: steps[id] ?? [] })
  }),

  // SSE event stream (must come before :id)
  http.get(`${API_BASE}/runs/:id/events`, ({ params }) => {
    const id = params.id as string
    const runEvents = events[id] ?? []

    const encoder = new TextEncoder()
    const stream = new ReadableStream({
      start(controller) {
        for (const event of runEvents) {
          controller.enqueue(encoder.encode(`data: ${JSON.stringify(event)}\n\n`))
        }
        // Keep connection open for SSE
      },
    })

    return new HttpResponse(stream, {
      headers: {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
      },
    })
  }),

  // SSE output stream (placeholder - returns empty stream for now)
  http.get(`${API_BASE}/runs/:id/output`, () => {
    const stream = new ReadableStream({
      start(controller) {
        // Output stream would send log chunks
        controller.enqueue(
          new TextEncoder().encode(
            `data: ${JSON.stringify({ step_id: 'step-001', offset: 0, content: 'Starting implementation...\n' })}\n\n`
          )
        )
      },
    })

    return new HttpResponse(stream, {
      headers: {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
      },
    })
  }),

  // Get single run (must come after more specific /runs/:id/* routes)
  http.get(`${API_BASE}/runs/:id`, ({ params }) => {
    const run = runs.find((r) => r.id === params.id)
    if (!run) {
      return new HttpResponse(null, { status: 404 })
    }
    return HttpResponse.json({ run })
  }),
]
