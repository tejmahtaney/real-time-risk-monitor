import { describe, expect, it } from 'vitest'

import type { DailyPnlRow } from '../api/generated/DailyPnlRow'
import { dailyPnlToCsv } from './storageCsv'

const row = (date: string, actual: number): DailyPnlRow => ({
  session_date: date,
  delta: 1.5,
  gamma: -0.25,
  vega: 2,
  theta: -3,
  rho: 0,
  higher_order: 0.125,
  dividends: 0,
  residual: 0.5,
  actual,
})

describe('dailyPnlToCsv', () => {
  it('writes a header and one full-precision line per day', () => {
    const csv = dailyPnlToCsv([row('2026-09-28', 1.23456789), row('2026-09-29', -40)])
    expect(csv).toBe(
      'session_date,delta,gamma,vega,theta,rho,higher_order,dividends,residual,actual\n' +
        '2026-09-28,1.5,-0.25,2,-3,0,0.125,0,0.5,1.23456789\n' +
        '2026-09-29,1.5,-0.25,2,-3,0,0.125,0,0.5,-40\n',
    )
  })

  it('is just the header for no rows', () => {
    expect(dailyPnlToCsv([]).trim().split('\n')).toHaveLength(1)
  })
})
