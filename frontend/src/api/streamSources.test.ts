import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { createStreamClient, wsUrlForSource } from './stream'
import { createRiskStore, useBacktestStore, useRiskStore } from '../store/riskStore'

class MockWebSocket {
  static instances: MockWebSocket[] = []
  url: string
  onopen: (() => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null

  constructor(url: string) {
    this.url = url
    MockWebSocket.instances.push(this)
  }

  message(data: unknown): void {
    this.onmessage?.({ data: JSON.stringify(data) })
  }

  close(): void {
    this.onclose?.()
  }
}

const snapshot = (seq: number): unknown => ({
  type: 'snapshot',
  snapshot: {
    seq,
    summary: { totals: {} },
    pnl: { portfolio: { today: {} } },
    limits: [],
    var: { headline_method: 'delta_gamma_mc', delta_gamma_mc: { var_99: 0 } },
  },
})

let fetchMock: ReturnType<typeof vi.fn>
const fetched = (): string[] => fetchMock.mock.calls.map((call) => call[0] as string)
const last = (): MockWebSocket => MockWebSocket.instances[MockWebSocket.instances.length - 1]!

describe('stream sources', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket)
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    useRiskStore.getState().reset()
    useBacktestStore.getState().reset()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('connects to /ws for live and /ws/backtests/{id} for a replay', () => {
    expect(wsUrlForSource({ kind: 'live' })).toMatch(/\/ws$/)
    expect(wsUrlForSource({ kind: 'backtest', id: '20260930T124101Z-bkcz' })).toMatch(
      /\/ws\/backtests\/20260930T124101Z-bkcz$/,
    )

    fetchMock.mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve([]) })
    const replay = createStreamClient({ source: { kind: 'backtest', id: 'run-1' } })
    replay.start()
    expect(last().url).toMatch(/\/ws\/backtests\/run-1$/)
    replay.stop()
    const live = createStreamClient({ source: { kind: 'live' } })
    live.start()
    expect(last().url).toMatch(/\/ws$/)
    live.stop()
  })

  it('does not bootstrap history or poll for a replay, but does for live', async () => {
    const replay = createStreamClient({ source: { kind: 'backtest', id: 'run-1' } })
    replay.start()
    last().close()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(
      fetched().filter((path) => path.startsWith('/api/history') || path === '/api/positions'),
    ).toEqual([])
    replay.stop()

    fetchMock.mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve([]) })
    const live = createStreamClient({ source: { kind: 'live' } })
    live.start()
    await vi.advanceTimersByTimeAsync(0)
    expect(fetched()).toContain('/api/history')
    live.stop()
  })

  it('reconnects a replay only while the run is in memory', async () => {
    const status = (in_memory: boolean, state: string): unknown => ({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ in_memory, state }),
    })
    const replay = createStreamClient({ source: { kind: 'backtest', id: 'run-1' } })
    replay.start()

    fetchMock.mockResolvedValueOnce(status(true, 'playing'))
    last().close()
    await vi.advanceTimersByTimeAsync(1_300)
    expect(MockWebSocket.instances).toHaveLength(2)

    // Finished, then freed from memory: no more attempts.
    fetchMock.mockResolvedValueOnce(status(true, 'finished'))
    last().close()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(MockWebSocket.instances).toHaveLength(2)
    replay.stop()
  })

  it('stops reconnecting once stopped, and a stopped socket close changes nothing', async () => {
    const replay = createStreamClient({
      source: { kind: 'backtest', id: 'run-1' },
      shouldReconnect: () => Promise.resolve(true),
    })
    replay.start()
    replay.stop()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(MockWebSocket.instances).toHaveLength(1)
  })

  it('keeps the live slice updating while a backtest slice is active', () => {
    const live = createStreamClient({ source: { kind: 'live' } })
    fetchMock.mockResolvedValue({ ok: true, status: 200, json: () => Promise.resolve([]) })
    live.start()
    const liveSocket = last()
    const replay = createStreamClient({ source: { kind: 'backtest', id: 'run-1' } })
    replay.start()
    const replaySocket = last()

    liveSocket.message(snapshot(1))
    replaySocket.message(snapshot(100))
    liveSocket.message(snapshot(2))
    replaySocket.message(snapshot(101))

    expect(useRiskStore.getState().snapshot?.seq).toBe(2)
    expect(useBacktestStore.getState().snapshot?.seq).toBe(101)

    replay.stop()
    liveSocket.message(snapshot(3))
    expect(useRiskStore.getState().snapshot?.seq).toBe(3)
    live.stop()
  })

  it('gives every store factory call independent state', () => {
    const a = createRiskStore()
    const b = createRiskStore()
    a.getState().setError('x')
    expect(b.getState().lastError).toBeNull()
  })
})
