import { describe, expect, it } from 'vitest'

import type { LimitStatus } from '../api/generated/LimitStatus'
import { computeChartScale, metricBounds, type MetricBounds } from './chartLimits'

function limitStatus(overrides: Partial<LimitStatus> = {}): LimitStatus {
  return {
    name: 'portfolio_dollar_delta',
    scope: { kind: 'portfolio' },
    value: 100_000,
    limit: 550_000,
    utilisation: 100_000 / 550_000,
    state: 'ok',
    ...overrides,
  }
}

describe('metricBounds', () => {
  it('gives symmetric upper and lower bounds for dollar delta', () => {
    const bounds = metricBounds([limitStatus({ name: 'portfolio_dollar_delta', limit: 550_000 })], 'dollar_delta')
    expect(bounds.upper).toEqual({ limit: 550_000, warningStart: 440_000 })
    expect(bounds.lower).toEqual({ limit: -550_000, warningStart: -440_000 })
  })

  it('gives symmetric bounds for dollar gamma 1% and vega too', () => {
    const gammaLimit = limitStatus({ name: 'portfolio_dollar_gamma1_pct', limit: 9_500 })
    expect(metricBounds([gammaLimit], 'dollar_gamma_1pct').upper?.limit).toBe(9_500)
    const vegaLimit = limitStatus({ name: 'portfolio_vega', limit: 2_200 })
    expect(metricBounds([vegaLimit], 'vega').upper?.limit).toBe(2_200)
  })

  it('gives theta only a lower bound, at minus the floor', () => {
    const thetaLimit = limitStatus({ name: 'portfolio_theta_floor', limit: 4_500 })
    const bounds = metricBounds([thetaLimit], 'theta')
    expect(bounds.upper).toBeNull()
    expect(bounds.lower).toEqual({ limit: -4_500, warningStart: -3_600 })
  })

  it('returns no bounds when the metric has no configured limit', () => {
    expect(metricBounds([], 'vega')).toEqual({ upper: null, lower: null })
  })

  it('ignores an underlying-scoped status with the same name', () => {
    const underlyingScoped = limitStatus({
      name: 'portfolio_dollar_delta',
      scope: { kind: 'underlying', symbol: 'TSLA' },
      limit: 74_000,
    })
    expect(metricBounds([underlyingScoped], 'dollar_delta')).toEqual({ upper: null, lower: null })
  })

  it('picks the right limit out of several configured ones', () => {
    const limits = [
      limitStatus({ name: 'portfolio_dollar_delta', limit: 550_000 }),
      limitStatus({ name: 'portfolio_vega', limit: 2_200 }),
      limitStatus({ name: 'gross_exposure', limit: 2_700_000 }),
    ]
    expect(metricBounds(limits, 'vega').upper?.limit).toBe(2_200)
  })
})

describe('computeChartScale', () => {
  const NO_BOUNDS: MetricBounds = { upper: null, lower: null }

  it('pads 10% beyond the data range on each side, not from zero, with no limit configured', () => {
    // Range is [400, 600] (200 wide); 10% of that is 20, so [380, 620].
    const scale = computeChartScale([400, 500, 600], NO_BOUNDS)
    expect(scale.domain).toEqual([380, 620])
    expect(scale.visibleBound).toBeNull()
    expect(scale.offscreenLimit).toBeNull()
  })

  it('pads a flat (constant, non-zero) series from its own magnitude, not the data range', () => {
    // range is 0, so pad = |500| * 10% = 50.
    expect(computeChartScale([500, 500, 500], NO_BOUNDS).domain).toEqual([450, 550])
  })

  it('pads a flat series at exactly zero by a fixed minimum span, not 10% of zero', () => {
    expect(computeChartScale([0, 0], NO_BOUNDS).domain).toEqual([-1, 1])
  })

  it('falls back to a fixed domain with no data at all, regardless of any configured bound', () => {
    const bounds: MetricBounds = { upper: { limit: 9_500, warningStart: 7_600 }, lower: null }
    const scale = computeChartScale([], bounds)
    expect(scale.domain).toEqual([-1, 1])
    expect(scale.visibleBound).toBeNull()
    expect(scale.offscreenLimit).toBeNull()
  })

  it('picks the upper side when the data leans positive, and draws nothing if only a lower bound is configured', () => {
    const bounds: MetricBounds = { upper: null, lower: { limit: -4_500, warningStart: -3_600 } }
    const scale = computeChartScale([100, 200, 300], bounds)
    expect(scale.domain).toEqual([80, 320])
    expect(scale.visibleBound).toBeNull()
    expect(scale.offscreenLimit).toBeNull()
  })

  it('picks the lower side when the data leans negative', () => {
    const bounds: MetricBounds = {
      upper: { limit: 4_500, warningStart: 3_600 },
      lower: { limit: -4_500, warningStart: -3_600 },
    }
    // dataMin=-3000, dataMax=-2000: positive extent 0, negative extent 3000 -> lower side.
    const scale = computeChartScale([-2_000, -2_500, -3_000], bounds)
    expect(scale.visibleBound).toEqual(bounds.lower)
  })

  it('breaks a tie between equal positive and negative extents towards the upper side', () => {
    // Deliberately very different upper/lower limits, so picking the wrong side would
    // be obvious: domain-only reach here is 60 (see the padding test above), and 90
    // sits right at 1.5x that, so the upper bound should draw; -500 is nowhere close
    // to visible, so if the lower bound were picked instead, nothing would draw.
    const bounds: MetricBounds = { upper: { limit: 90, warningStart: 72 }, lower: { limit: -500, warningStart: -400 } }
    const scale = computeChartScale([-50, 50], bounds)
    expect(scale.visibleBound).toEqual(bounds.upper)
  })

  it("does not flatten the chart the way anchoring at ±limit did: gamma near $194 with a $9.5k limit stays on a tight axis, and shows the limit as an offscreen label instead", () => {
    const bounds: MetricBounds = { upper: { limit: 9_500, warningStart: 7_600 }, lower: { limit: -9_500, warningStart: -7_600 } }
    const scale = computeChartScale([190, 194, 198], bounds)
    expect(scale.domain).toEqual([189.2, 198.8])
    expect(scale.visibleBound).toBeNull()
    expect(scale.offscreenLimit).toBe(9_500)
  })

  it('draws the limit and band when they sit within 1.5x of the data-only domain, extending only the active side', () => {
    // Mirrors the sample book's static stock: dollar delta hovers near its own $550k
    // limit, not far from it, so the line should actually draw.
    const bounds: MetricBounds = {
      upper: { limit: 550_000, warningStart: 440_000 },
      lower: { limit: -550_000, warningStart: -440_000 },
    }
    const scale = computeChartScale([490_000, 500_000, 510_000], bounds)
    // data-only domain would be [488_000, 512_000]; domainReach 512_000, limit 550_000
    // is within 1.5x (768_000), so the upper side extends to include it (plus 10%
    // headroom on the extended reach: 550_000 * 1.1 = 605_000); the lower side keeps
    // its data-only pad untouched.
    expect(scale.domain).toEqual([488_000, 605_000])
    expect(scale.visibleBound).toEqual(bounds.upper)
    expect(scale.offscreenLimit).toBeNull()
  })

  it('is visible exactly at 1.5x the domain reach, and offscreen just past it', () => {
    // values=[0, 100]: range 100, pad 10 -> domain [-10, 110], domainReach (upper) = 110.
    const values = [0, 100]
    const visibleBounds: MetricBounds = { upper: { limit: 165, warningStart: 132 }, lower: null }
    expect(computeChartScale(values, visibleBounds).visibleBound).not.toBeNull()

    const offscreenBounds: MetricBounds = { upper: { limit: 166, warningStart: 132.8 }, lower: null }
    const scale = computeChartScale(values, offscreenBounds)
    expect(scale.visibleBound).toBeNull()
    expect(scale.offscreenLimit).toBe(166)
  })
})
