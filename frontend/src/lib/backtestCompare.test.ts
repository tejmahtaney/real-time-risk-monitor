import { describe, expect, it } from 'vitest'

import type { DailyRow } from '../api/generated/DailyRow'
import { overlaySeries } from './backtestCompare'

const row = (date: string, cumulative_pnl: number): DailyRow => ({
  date,
  delta: 0,
  gamma: 0,
  vega: 0,
  theta: 0,
  rho: 0,
  higher_order: 0,
  dividends: 0,
  residual: 0,
  actual: 0,
  cumulative_pnl,
  var_99: 0,
  exception: false,
  breaches: 0,
})

describe('overlaySeries', () => {
  it('unions the dates of two runs and leaves gaps as null', () => {
    const out = overlaySeries([row('2026-01-05', 1), row('2026-01-06', 2)], [row('2026-01-06', 9), row('2026-01-07', 8)])
    expect(out).toEqual([
      { date: '2026-01-05', a: 1, b: null },
      { date: '2026-01-06', a: 2, b: 9 },
      { date: '2026-01-07', a: null, b: 8 },
    ])
  })
})
