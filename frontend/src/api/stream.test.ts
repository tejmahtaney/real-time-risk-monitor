import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useRiskStore } from '../store/riskStore'
import { startStream, stopStreamForTests } from './stream'

/** A minimal, synchronously-controllable stand-in for the browser `WebSocket`, so
 * tests can drive open/message/close exactly when they want to, with no real
 * network. */
class MockWebSocket {
  static instances: MockWebSocket[] = []
  url: string
  readyState = 0
  onopen: (() => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null

  constructor(url: string) {
    this.url = url
    MockWebSocket.instances.push(this)
  }

  triggerOpen(): void {
    this.readyState = 1
    this.onopen?.()
  }

  triggerMessage(data: unknown): void {
    this.onmessage?.({ data: JSON.stringify(data) })
  }

  triggerClose(): void {
    this.readyState = 3
    this.onclose?.()
  }

  close(): void {
    this.triggerClose()
  }
}

function latestSocket(): MockWebSocket {
  const socket = MockWebSocket.instances[MockWebSocket.instances.length - 1]
  if (!socket) throw new Error('no WebSocket was constructed')
  return socket
}

let fetchMock: ReturnType<typeof vi.fn>

function mockFetchJson(body: unknown, status = 200): void {
  fetchMock.mockResolvedValueOnce({
    ok: status < 400,
    status,
    json: () => Promise.resolve(body),
  })
}

function fetchedPaths(): string[] {
  return fetchMock.mock.calls.map((call) => call[0] as string)
}

function testWsUrl(): string {
  return 'ws://test/ws'
}

describe('stream', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket)
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    useRiskStore.getState().reset()
    stopStreamForTests()
  })

  afterEach(() => {
    stopStreamForTests()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('opens exactly one socket even if startStream is called twice (StrictMode double-mount)', () => {
    startStream(testWsUrl)
    startStream(testWsUrl)
    expect(MockWebSocket.instances).toHaveLength(1)
  })

  it('marks the store open on connect and applies an incoming snapshot', () => {
    startStream(testWsUrl)
    latestSocket().triggerOpen()
    expect(useRiskStore.getState().status).toBe('open')

    latestSocket().triggerMessage({
      type: 'snapshot',
      snapshot: { seq: 3, summary: { totals: {} }, pnl: { portfolio: { today: {} } }, limits: [] },
    })
    expect(useRiskStore.getState().snapshot?.seq).toBe(3)
  })

  it('falls back to polling once the socket closes, and stops once it reopens', async () => {
    startStream(testWsUrl)
    latestSocket().triggerOpen()

    mockFetchJson({ totals: {}, concentration: [], limits: [], compute_micros: 1 })
    mockFetchJson([])
    latestSocket().triggerClose()
    // The first poll fires immediately on entering the polling state.
    await vi.advanceTimersByTimeAsync(0)
    expect(useRiskStore.getState().status).toBe('polling')
    expect(fetchedPaths()).toContain('/api/risk-summary')
    expect(fetchedPaths()).toContain('/api/positions')

    const callsAfterFirstPoll = fetchMock.mock.calls.length
    mockFetchJson({ totals: {}, concentration: [], limits: [], compute_micros: 1 })
    mockFetchJson([])
    await vi.advanceTimersByTimeAsync(2_000)
    expect(fetchMock.mock.calls.length).toBeGreaterThan(callsAfterFirstPoll)

    // Reopen: polling should stop, so no further polling fetches happen even after
    // more time passes.
    latestSocket().triggerOpen()
    const pollingCallsAfterReopen = fetchedPaths().filter(
      (path) => path === '/api/risk-summary' || path === '/api/positions',
    ).length
    await vi.advanceTimersByTimeAsync(4_000)
    const pollingCallsLater = fetchedPaths().filter(
      (path) => path === '/api/risk-summary' || path === '/api/positions',
    ).length
    expect(pollingCallsLater).toBe(pollingCallsAfterReopen)
  })

  it('backs off exponentially between reconnect attempts, with jitter kept within bounds', async () => {
    startStream(testWsUrl)
    latestSocket().triggerOpen()

    mockFetchJson({ totals: {}, concentration: [], limits: [], compute_micros: 1 })
    mockFetchJson([])
    latestSocket().triggerClose()
    expect(MockWebSocket.instances).toHaveLength(1)

    // Just under 1s (plus up to 20% jitter): still no second attempt this early.
    await vi.advanceTimersByTimeAsync(900)
    expect(MockWebSocket.instances).toHaveLength(1)

    // By 1.2s (1s plus the maximum jitter), the first retry must have fired.
    await vi.advanceTimersByTimeAsync(300)
    expect(MockWebSocket.instances).toHaveLength(2)

    // The second attempt also fails immediately; the next backoff should be ~2s, not ~1s.
    mockFetchJson({ totals: {}, concentration: [], limits: [], compute_micros: 1 })
    mockFetchJson([])
    latestSocket().triggerClose()
    await vi.advanceTimersByTimeAsync(1_700)
    expect(MockWebSocket.instances).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(700)
    expect(MockWebSocket.instances).toHaveLength(3)
  })

  it('backfills the full buffered history once on load, before the socket even opens', async () => {
    mockFetchJson([{ seq: 1 }, { seq: 2 }])
    startStream(testWsUrl)
    await vi.advanceTimersByTimeAsync(0)

    expect(fetchedPaths()).toContain('/api/history')
    expect(useRiskStore.getState().history.map((p) => p.seq)).toEqual([1, 2])
  })

  it('backfills history since the last known seq on reconnect', async () => {
    startStream(testWsUrl)
    latestSocket().triggerOpen()
    latestSocket().triggerMessage({
      type: 'snapshot',
      snapshot: { seq: 42, summary: { totals: {} }, pnl: { portfolio: { today: {} } }, limits: [] },
    })

    mockFetchJson({ totals: {}, concentration: [], limits: [], compute_micros: 1 })
    mockFetchJson([])
    latestSocket().triggerClose()
    await vi.advanceTimersByTimeAsync(1_300)

    mockFetchJson([{ seq: 43 }])
    latestSocket().triggerOpen()
    await vi.advanceTimersByTimeAsync(0)

    expect(fetchedPaths()).toContain('/api/history?since=42')
  })

  it('logs an unknown message type once and otherwise ignores it', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    startStream(testWsUrl)
    latestSocket().triggerOpen()

    latestSocket().triggerMessage({ type: 'from_the_future' })
    latestSocket().triggerMessage({ type: 'from_the_future' })

    expect(warn).toHaveBeenCalledTimes(1)
    warn.mockRestore()
  })
})
