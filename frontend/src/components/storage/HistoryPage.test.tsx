import { render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { SessionSummary } from '../../api/generated/SessionSummary'
import { api } from '../../api/rest'
import { HistoryPage } from './HistoryPage'
import { SessionDetail } from './SessionDetail'

afterEach(() => vi.restoreAllMocks())

const summary = (id: string, overrides: Partial<SessionSummary> = {}): SessionSummary => ({
  id,
  start: '2026-10-01T13:00:00Z',
  end: null,
  simulated_days: 0,
  size_bytes: 1024,
  git_commit: 'abcdef1234567',
  total_pnl: null,
  running: false,
  ...overrides,
})

describe('HistoryPage', () => {
  it('badges the running session and keeps "cut short" for the ones that crashed', async () => {
    vi.spyOn(api, 'sessions').mockResolvedValue([
      summary('live', { start: '2026-10-02T13:00:00Z', running: true }),
      summary('crashed'),
      summary('done', { end: '2026-10-01T14:00:00Z' }),
    ])
    render(<HistoryPage />)

    const live = await screen.findByTestId('session-live')
    expect(within(live).getByText('running')).toBeInTheDocument()
    expect(within(live).queryByText('cut short')).not.toBeInTheDocument()
    expect(within(screen.getByTestId('session-crashed')).getByText('cut short')).toBeInTheDocument()
    expect(within(screen.getByTestId('session-done')).queryByText('cut short')).not.toBeInTheDocument()
  })
})

describe('SessionDetail', () => {
  const meta = {
    session_id: 'live',
    start: '2026-10-02T13:00:00Z',
    end: null,
    seed: 1,
    speed: 1,
    git_commit: 'abcdef1234',
    dropped_records: 0,
  }

  it.each([
    [true, 'running', 'cut short'],
    [false, 'cut short', 'running'],
  ])('with running=%s shows %s and not %s', async (running, shown, hidden) => {
    vi.spyOn(api, 'sessionDetail').mockResolvedValue({ meta, daily_pnl: [], running } as never)
    vi.spyOn(api, 'sessionHistory').mockResolvedValue([])
    vi.spyOn(api, 'sessionEvents').mockResolvedValue([])
    render(<SessionDetail id="live" onBack={() => undefined} />)

    expect(await screen.findByText(shown)).toBeInTheDocument()
    expect(screen.queryByText(hidden)).not.toBeInTheDocument()
  })
})
