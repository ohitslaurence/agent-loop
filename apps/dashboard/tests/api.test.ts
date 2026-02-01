import { describe, it, expect } from 'vitest'
import { healthCheck, listRuns, getRun, listSteps } from '@/lib/api'

describe('API client', () => {
  it('healthCheck returns ok status', async () => {
    const result = await healthCheck()
    expect(result.status).toBe('ok')
  })

  it('listRuns returns runs array', async () => {
    const runs = await listRuns()
    expect(Array.isArray(runs)).toBe(true)
    expect(runs.length).toBeGreaterThan(0)
    expect(runs[0]).toHaveProperty('id')
    expect(runs[0]).toHaveProperty('name')
    expect(runs[0]).toHaveProperty('status')
  })

  it('getRun returns a single run', async () => {
    const run = await getRun('run-001')
    expect(run.id).toBe('run-001')
    expect(run.name).toBe('Add user authentication')
    expect(run.status).toBe('Running')
  })

  it('getRun throws for non-existent run', async () => {
    await expect(getRun('non-existent')).rejects.toThrow()
  })

  it('listSteps returns steps array', async () => {
    const steps = await listSteps('run-001')
    expect(Array.isArray(steps)).toBe(true)
    expect(steps.length).toBeGreaterThan(0)
    expect(steps[0]).toHaveProperty('phase')
    expect(steps[0]).toHaveProperty('status')
  })
})
