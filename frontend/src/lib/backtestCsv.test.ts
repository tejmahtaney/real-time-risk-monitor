import { describe, expect, it } from 'vitest'

import type { DailyRow } from '../api/generated/DailyRow'
import { dailyRowsToCsv } from './backtestCsv'
import { cumulativeSeries, drawdownSeries } from './backtestSeries'

function row(
  date: string,
  actual: number,
  cumulative: number,
  overrides: Partial<DailyRow> = {},
): DailyRow {
  return {
    date,
    delta: actual,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
    actual,
    cumulative_pnl: cumulative,
    var_99: 1000.5,
    exception: false,
    breaches: 0,
    ...overrides,
  }
}

describe('dailyRowsToCsv', () => {
  it('writes a header and one line per date with full-precision values', () => {
    const csv = dailyRowsToCsv([
      row('2026-06-04', 12.345678, 12.345678),
      row('2026-06-05', -3, 9.345678, { exception: true, breaches: 2 }),
    ])
    expect(csv.split('\n')).toEqual([
      'date,delta,gamma,vega,theta,rho,higher_order,dividends,residual,actual,cumulative_pnl,var_99,exception,breaches',
      '2026-06-04,12.345678,0,0,0,0,0,0,0,12.345678,12.345678,1000.5,false,0',
      '2026-06-05,-3,0,0,0,0,0,0,0,-3,9.345678,1000.5,true,2',
      '',
    ])
  })

  it('writes just the header for no rows', () => {
    expect(dailyRowsToCsv([]).trim().split('\n')).toHaveLength(1)
  })
})

describe('result series', () => {
  const rows = [row('d1', 10, 10), row('d2', -4, 6), row('d3', -8, -2), row('d4', 5, 3)]

  it('accumulates each bucket', () => {
    expect(cumulativeSeries(rows).map((p) => p.delta)).toEqual([10, 6, -2, 3])
  })

  it('measures the drawdown below the running peak, starting from zero', () => {
    expect(drawdownSeries(rows).map((p) => p.drawdown)).toEqual([0, -4, -12, -7])
    expect(drawdownSeries([row('d1', -5, -5)]).map((p) => p.drawdown)).toEqual([-5])
  })
})
