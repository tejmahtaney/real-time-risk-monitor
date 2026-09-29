import { describe, expect, it } from 'vitest'

import type { Concentration } from '../api/generated/Concentration'
import type { UnderlyingRisk } from '../api/generated/UnderlyingRisk'
import { allocationTotal, grossExposureSlices, vegaSlices } from './allocation'

function underlyingRisk(overrides: Partial<UnderlyingRisk> = {}): UnderlyingRisk {
  return {
    spot: 100,
    share_delta: 0,
    dollar_delta: 0,
    share_gamma: 0,
    dollar_gamma_1pct: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    market_value: 0,
    unrealised_pnl: 0,
    realised_pnl: 0,
    gross_exposure: 0,
    contract_notional: 0,
    position_count: 1,
    ...overrides,
  }
}

describe('grossExposureSlices', () => {
  it('maps concentration entries straight through, order preserved', () => {
    const concentration: Concentration[] = [
      { symbol: 'SPY', gross_exposure: 400_000, fraction: 0.8 },
      { symbol: 'AAPL', gross_exposure: 100_000, fraction: 0.2 },
    ]
    expect(grossExposureSlices(concentration)).toEqual([
      { symbol: 'SPY', value: 400_000, fraction: 0.8 },
      { symbol: 'AAPL', value: 100_000, fraction: 0.2 },
    ])
  })

  it('is empty when concentration is empty', () => {
    expect(grossExposureSlices([])).toEqual([])
  })
})

describe('vegaSlices', () => {
  it('takes the absolute value of vega and sorts largest share first', () => {
    const slices = vegaSlices({
      AAPL: underlyingRisk({ vega: -300 }),
      SPY: underlyingRisk({ vega: 700 }),
    })
    expect(slices).toEqual([
      { symbol: 'SPY', value: 700, fraction: 0.7 },
      { symbol: 'AAPL', value: 300, fraction: 0.3 },
    ])
  })

  it('breaks ties in symbol order', () => {
    const slices = vegaSlices({
      TSLA: underlyingRisk({ vega: 100 }),
      MSFT: underlyingRisk({ vega: -100 }),
    })
    expect(slices.map((s) => s.symbol)).toEqual(['MSFT', 'TSLA'])
  })

  it('gives every symbol fraction 0 when total |vega| is zero, not NaN', () => {
    const slices = vegaSlices({ SPY: underlyingRisk({ vega: 0 }), AAPL: underlyingRisk({ vega: 0 }) })
    expect(slices.every((s) => s.fraction === 0)).toBe(true)
  })

  it('is empty for an empty book', () => {
    expect(vegaSlices({})).toEqual([])
  })
})

describe('allocationTotal', () => {
  it('sums every slice value', () => {
    expect(allocationTotal([{ symbol: 'SPY', value: 400, fraction: 0.8 }, { symbol: 'AAPL', value: 100, fraction: 0.2 }])).toBe(500)
  })

  it('is zero for no slices', () => {
    expect(allocationTotal([])).toBe(0)
  })
})
