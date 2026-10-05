import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { leaseRun } from './runLease'

let fetchMock: ReturnType<typeof vi.fn>
const deletes = (): string[] =>
  fetchMock.mock.calls
    .filter((call) => (call[1] as RequestInit).method === 'DELETE')
    .map((call) => call[0] as string)

describe('leaseRun', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 204 })
    vi.stubGlobal('fetch', fetchMock)
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('does not free the run on a StrictMode unmount and re-mount', async () => {
    const first = leaseRun('run-a')
    first() // StrictMode's throw-away cleanup
    const second = leaseRun('run-a') // the immediate re-mount
    await vi.advanceTimersByTimeAsync(50)
    expect(deletes()).toEqual([])

    second() // a real route change
    await vi.advanceTimersByTimeAsync(50)
    expect(deletes()).toEqual(['/api/backtests/run-a'])
  })

  it('frees a run once on a real unmount', async () => {
    leaseRun('run-b')()
    await vi.advanceTimersByTimeAsync(50)
    expect(deletes()).toEqual(['/api/backtests/run-b'])
  })

  it('frees held runs with a keepalive DELETE when the page unloads', () => {
    leaseRun('run-c')
    window.dispatchEvent(new Event('pagehide'))
    const call = fetchMock.mock.calls.find((c) => c[0] === '/api/backtests/run-c')
    expect(call?.[1]).toMatchObject({ method: 'DELETE', keepalive: true })
  })
})
