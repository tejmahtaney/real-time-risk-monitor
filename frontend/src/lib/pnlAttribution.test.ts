import { describe, expect, it } from 'vitest'

import type { PnlBuckets } from '../api/generated/PnlBuckets'
import type { PnlReport } from '../api/generated/PnlReport'
import { bucketRows, portfolioBucketsForPeriod, underlyingStacks } from './pnlAttribution'

function buckets(overrides: Partial<PnlBuckets> = {}): PnlBuckets {
  return { delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, higher_order: 0, residual: 0, actual: 0, ...overrides }
}

function report(overrides: Partial<PnlReport> = {}): PnlReport {
  return {
    session_date: '2026-09-29',
    portfolio: { today: buckets(), since_start: buckets() },
    by_underlying: {},
    by_position: {},
    previous_day: null,
    ...overrides,
  }
}

describe('bucketRows', () => {
  it('lists every bucket in the fixed chart order', () => {
    const rows = bucketRows(buckets({ delta: 100, residual: 5, actual: 95 }))
    expect(rows.map((r) => r.key)).toEqual([
      'delta',
      'gamma',
      'vega',
      'theta',
      'rho',
      'higher_order',
      'residual',
      'actual',
    ])
    expect(rows.find((r) => r.key === 'delta')?.value).toBe(100)
    expect(rows.find((r) => r.key === 'actual')?.value).toBe(95)
  })
})

describe('portfolioBucketsForPeriod', () => {
  const r = report({
    portfolio: { today: buckets({ delta: 1 }), since_start: buckets({ delta: 2 }) },
    previous_day: {
      session_date: '2026-09-28',
      portfolio: buckets({ delta: 3 }),
      by_underlying: {},
      by_position: {},
    },
  })

  it("reads today's buckets", () => {
    expect(portfolioBucketsForPeriod(r, 'today')).toEqual(buckets({ delta: 1 }))
  })

  it('reads since-start buckets', () => {
    expect(portfolioBucketsForPeriod(r, 'since_start')).toEqual(buckets({ delta: 2 }))
  })

  it("reads the previous day's buckets", () => {
    expect(portfolioBucketsForPeriod(r, 'previous_day')).toEqual(buckets({ delta: 3 }))
  })

  it('is null for previous day before the first session roll', () => {
    expect(portfolioBucketsForPeriod(report(), 'previous_day')).toBeNull()
  })
})

describe('underlyingStacks', () => {
  it('lists every underlying in alphabetical order with the stacked keys only', () => {
    const r = report({
      by_underlying: {
        SPY: { today: buckets({ delta: 10, actual: 999 }), since_start: buckets() },
        AAPL: { today: buckets({ delta: 5 }), since_start: buckets() },
      },
    })
    const stacks = underlyingStacks(r, 'today')
    expect(stacks.map((s) => s.symbol)).toEqual(['AAPL', 'SPY'])
    const spyStack = stacks[1]
    expect(spyStack?.values).toEqual({
      delta: 10,
      gamma: 0,
      vega: 0,
      theta: 0,
      rho: 0,
      higher_order: 0,
      residual: 0,
    })
    expect(spyStack?.values.actual).toBeUndefined()
  })

  it('is empty for previous day before the first session roll', () => {
    expect(underlyingStacks(report(), 'previous_day')).toEqual([])
  })

  it("reads the previous day's per-underlying buckets once rolled", () => {
    const r = report({
      previous_day: {
        session_date: '2026-09-28',
        portfolio: buckets(),
        by_underlying: { TSLA: buckets({ vega: 42 }) },
        by_position: {},
      },
    })
    const stacks = underlyingStacks(r, 'previous_day')
    expect(stacks).toEqual([{ symbol: 'TSLA', values: { delta: 0, gamma: 0, vega: 42, theta: 0, rho: 0, higher_order: 0, residual: 0 } }])
  })
})
