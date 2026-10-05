import { act, render, screen } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { BacktestStatus } from '../../api/generated/BacktestStatus'
import { useBacktestStore } from '../../store/riskStore'
import { ReplayView } from './ReplayView'

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
  close(): void {}
}

const STATUS: BacktestStatus = {
  id: 'run-1',
  state: 'paused',
  in_memory: true,
  speed: 5,
  roll_expired: true,
  source: { kind: 'archive' },
  start_date: '2026-06-04',
  last_date: '2026-09-28',
  current_date: '2026-06-10',
  steps_done: 4,
  steps_total: 80,
  progress: 0.05,
}

let fetchMock: ReturnType<typeof vi.fn>
const deletes = (): string[] =>
  fetchMock.mock.calls
    .filter((call) => (call[1] as RequestInit | undefined)?.method === 'DELETE')
    .map((call) => call[0] as string)

describe('ReplayView', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket)
    fetchMock = vi.fn((path: string) =>
      Promise.resolve({
        ok: true,
        status: path.endsWith('/run-1') ? 200 : 204,
        json: () => Promise.resolve(STATUS),
      }),
    )
    vi.stubGlobal('fetch', fetchMock)
    useBacktestStore.getState().reset()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('shows progress and date from the run status, not from the live snapshot', async () => {
    render(<ReplayView id="run-1" onClose={vi.fn()} />)
    await act(() => vi.advanceTimersByTimeAsync(10))
    expect(screen.getByTestId('replay-progress')).toHaveTextContent(
      '2026-06-10: 4 of 80 dates (paused)',
    )
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '5')
    expect(screen.getByRole('button', { name: 'Play' })).toBeEnabled()
  })

  it('connects its stream to the run and keeps it after a StrictMode double mount', async () => {
    const view = render(
      <StrictMode>
        <ReplayView id="run-1" onClose={vi.fn()} />
      </StrictMode>,
    )
    await act(() => vi.advanceTimersByTimeAsync(50))
    expect(
      MockWebSocket.instances.every((socket) => socket.url.endsWith('/ws/backtests/run-1')),
    ).toBe(true)
    // StrictMode's throw-away unmount must not free the run.
    expect(deletes()).toEqual([])

    view.unmount()
    await act(() => vi.advanceTimersByTimeAsync(50))
    // A real unmount frees it, once.
    expect(deletes()).toEqual(['/api/backtests/run-1'])
  })
})
