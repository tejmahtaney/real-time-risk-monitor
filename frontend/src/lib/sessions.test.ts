import { describe, expect, it } from 'vitest'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { SessionEvent } from '../api/generated/SessionEvent'
import type { SessionSummary } from '../api/generated/SessionSummary'
import {
  alignSessions,
  cumulativePnl,
  describeEvent,
  newestFirst,
  rangeBounds,
  sessionDates,
  sessionRowView,
} from './sessions'

const summary = (overrides: Partial<SessionSummary> = {}): SessionSummary => ({
  id: 's1',
  start: '2026-10-01T13:00:00Z',
  end: '2026-10-01T14:00:00Z',
  simulated_days: 3,
  size_bytes: 2048,
  git_commit: 'abcdef1234567',
  total_pnl: 1234,
  running: false,
  ...overrides,
})

const point = (sim_time: string, today: number, delta = 0): HistoryPoint => ({
  seq: 0,
  sim_time,
  dollar_delta: delta,
  dollar_gamma_1pct: 0,
  vega: 0,
  theta: 0,
  gross_exposure: 0,
  market_value: 0,
  pnl_today: {
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
    actual: today,
  },
  worst_limit_utilisation: 0,
  var_99: null,
})

describe('sessionRowView', () => {
  it('formats a finished session', () => {
    const view = sessionRowView(summary())
    expect(view.cutShort).toBe(false)
    expect(view.end).toBe('Oct 1, 10:00 ET')
    expect(view.start).toBe('Oct 1, 09:00 ET')
    expect(view.commit).toBe('abcdef1')
    expect(view.size).toBe('2.0 KB')
    expect(view.pnl).toBe('$1.2k')
  })

  it('says cut short with a dash for a session with no end or P&L', () => {
    const view = sessionRowView(summary({ end: null, total_pnl: null }))
    expect(view.cutShort).toBe(true)
    expect(view.end).toBe('cut short')
    expect(view.pnl).toBe('—')
  })

  it('says running, not cut short, for the engine\'s current session', () => {
    const view = sessionRowView(summary({ end: null, total_pnl: null, running: true }))
    expect(view.running).toBe(true)
    expect(view.cutShort).toBe(false)
    expect(view.end).toBe('running')
  })

  it('sorts newest first', () => {
    const sorted = newestFirst([
      summary({ id: 'old', start: '2026-09-01T00:00:00Z' }),
      summary({ id: 'new', start: '2026-10-01T00:00:00Z' }),
    ])
    expect(sorted.map((s) => s.id)).toEqual(['new', 'old'])
  })
})

describe('session dates and ranges', () => {
  const points = [
    point('2026-09-28T14:00:00Z', 0),
    point('2026-09-28T19:00:00Z', 0),
    point('2026-09-29T14:00:00Z', 0),
    point('2026-09-29T20:00:00Z', 0),
  ]

  it('groups by New York date', () => {
    const dates = sessionDates(points)
    expect(dates.map((d) => d.date)).toEqual(['2026-09-28', '2026-09-29'])
    expect(dates[1]?.last).toBe('2026-09-29T20:00:00Z')
  })

  it('turns two dates into from and to bounds', () => {
    const dates = sessionDates(points)
    expect(rangeBounds(dates, '2026-09-28', '2026-09-29')).toEqual({
      from: '2026-09-28T14:00:00Z',
      to: '2026-09-29T20:00:00Z',
    })
    expect(rangeBounds(dates, '2026-09-29', '2026-09-28')).toBeNull()
    expect(rangeBounds(dates, '2026-01-01', '2026-09-28')).toBeNull()
  })
})

describe('cumulativePnl', () => {
  it('carries each day’s last P&L into the next day', () => {
    const points = [
      point('2026-09-28T14:00:00Z', 10),
      point('2026-09-28T19:00:00Z', 30),
      point('2026-09-29T14:00:00Z', 5),
      point('2026-09-29T19:00:00Z', -5),
    ]
    expect(cumulativePnl(points)).toEqual([10, 30, 35, 25])
  })
})

describe('alignSessions', () => {
  it('aligns by time since each session’s first point, not by wall time', () => {
    // Same simulated start date, but the second session started an hour into the day's
    // data and a different clock date altogether.
    const a = [point('2026-09-28T14:00:00Z', 1, 10), point('2026-09-28T15:00:00Z', 2, 20)]
    const b = [point('2026-06-01T14:30:00Z', 5, 100), point('2026-06-01T15:30:00Z', 7, 200)]
    const aligned = alignSessions(a, b, 'dollar_delta')
    expect(aligned).toEqual([
      { hours: 0, pnlA: 1, pnlB: 5, greekA: 10, greekB: 100 },
      { hours: 1, pnlA: 2, pnlB: 7, greekA: 20, greekB: 200 },
    ])
  })

  it('keeps each session’s own sample times, with null where the other has none', () => {
    const a = [point('2026-09-28T14:00:00Z', 1), point('2026-09-28T16:00:00Z', 3)]
    const b = [point('2026-09-28T14:00:00Z', 2), point('2026-09-28T15:00:00Z', 4)]
    const aligned = alignSessions(a, b, 'vega')
    expect(aligned.map((p) => p.hours)).toEqual([0, 1, 2])
    expect(aligned[1]).toMatchObject({ pnlA: null, pnlB: 4 })
    expect(aligned[2]).toMatchObject({ pnlA: 3, pnlB: null })
  })

  it('handles an empty session', () => {
    expect(alignSessions([], [point('2026-09-28T14:00:00Z', 1)], 'vega')).toHaveLength(1)
    expect(alignSessions([], [], 'vega')).toEqual([])
  })
})

describe('describeEvent', () => {
  const alert = (
    scope: SessionEvent & { type: 'alert' } extends infer A
      ? A extends { scope: infer S }
        ? S
        : never
      : never,
  ): SessionEvent => ({
    type: 'alert',
    id: 1,
    wall_time: '2026-10-01T13:00:00Z',
    sim_time: '2026-10-01T13:00:00Z',
    name: 'underlying_dollar_delta',
    scope,
    from: 'ok',
    to: 'breach',
    value: 1.2345e6,
    limit: 1e6,
  })

  it('uses the alerts pane labels and money format, never scientific notation', () => {
    const text = describeEvent(alert({ kind: 'portfolio' }))
    expect(text).toBe('Underlying $ delta (Portfolio) ok → breach, $1.2M / $1.0M')
    expect(text).not.toMatch(/e[+-]?\d/)
  })

  it('shows the symbol for a per-underlying limit', () => {
    expect(describeEvent(alert({ kind: 'underlying', symbol: 'AAPL' }))).toContain('(AAPL)')
  })
})
