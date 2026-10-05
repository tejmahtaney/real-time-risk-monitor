import { describe, expect, it } from 'vitest'

import type { DailyRow } from '../api/generated/DailyRow'
import { negKey, posKey, stackedDomain, stackedSeries } from './backtestSeries'

function row(date: string, delta: number, gamma: number, cumulative: number): DailyRow {
  return {
    date,
    delta,
    gamma,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
    actual: delta + gamma,
    cumulative_pnl: cumulative,
    var_99: 0,
    exception: false,
    breaches: 0,
  }
}

describe('stackedSeries', () => {
  it('puts each running bucket total on its own side of zero', () => {
    const [point] = stackedSeries([row('d1', 100, -40, 60)])
    expect(point?.[posKey('delta')]).toBe(100)
    expect(point?.[negKey('delta')]).toBe(0)
    expect(point?.[posKey('gamma')]).toBe(0)
    expect(point?.[negKey('gamma')]).toBe(-40)
  })
})

describe('stackedDomain', () => {
  it('fits the stacks and the cumulative line, not the sum of absolute values', () => {
    const points = stackedSeries([row('d1', 100, -40, 60), row('d2', 100, -40, 120)])
    const [lo, hi] = stackedDomain(points)
    // Positive stack peaks at 200, negative at -80: the old single stack would have reached 280.
    expect(hi).toBeGreaterThanOrEqual(200)
    expect(hi).toBeLessThan(280)
    expect(lo).toBeLessThanOrEqual(-80)
    expect(lo).toBeGreaterThan(-120)
  })

  it('is [0, 0] for no data', () => {
    expect(stackedDomain([])).toEqual([0, 0])
  })
})
