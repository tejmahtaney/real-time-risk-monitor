import { describe, expect, it } from 'vitest'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import { capHistory, filterWindow } from './chartWindow'

function point(secondsFromEpoch: number, seq: number): HistoryPoint {
  return {
    seq,
    sim_time: new Date(secondsFromEpoch * 1000).toISOString(),
    dollar_delta: 0,
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
      residual: 0,
      actual: 0,
    },
    worst_limit_utilisation: 0,
    var_99: null,
  }
}

describe('filterWindow', () => {
  const history = [point(0, 1), point(60, 2), point(300, 3), point(600, 4), point(900, 5)]

  it("returns everything for 'all'", () => {
    expect(filterWindow(history, 'all')).toEqual(history)
  })

  it("keeps only points within 5 minutes of the last point's sim time", () => {
    // Last point is at t=900s; 5m window is [600, 900].
    expect(filterWindow(history, '5m').map((p) => p.seq)).toEqual([4, 5])
  })

  it("keeps only points within 15 minutes of the last point's sim time", () => {
    // Last point is at t=900s; 15m window is [0, 900] (900s = 15m), so everything qualifies.
    expect(filterWindow(history, '15m').map((p) => p.seq)).toEqual([1, 2, 3, 4, 5])
  })

  it('returns an empty array unchanged', () => {
    expect(filterWindow([], '5m')).toEqual([])
  })

  it('is a boundary-inclusive window (a point exactly at the cutoff is kept)', () => {
    const boundary = [point(0, 1), point(300, 2)] // 5m = 300s apart, exactly at the cutoff.
    expect(filterWindow(boundary, '5m').map((p) => p.seq)).toEqual([1, 2])
  })

  it('falls back to the full history when the last sim_time cannot be parsed', () => {
    const broken = [point(0, 1), { ...point(60, 2), sim_time: 'not-a-date' }]
    expect(filterWindow(broken, '5m')).toEqual(broken)
  })
})

describe('capHistory', () => {
  it('keeps the most recent points when over the cap', () => {
    const history = [point(0, 1), point(1, 2), point(2, 3), point(3, 4)]
    expect(capHistory(history, 2).map((p) => p.seq)).toEqual([3, 4])
  })

  it('is a no-op when already within the cap', () => {
    const history = [point(0, 1), point(1, 2)]
    expect(capHistory(history, 5)).toEqual(history)
  })

  it('is a no-op exactly at the cap', () => {
    const history = [point(0, 1), point(1, 2)]
    expect(capHistory(history, 2)).toEqual(history)
  })
})
