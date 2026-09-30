import { describe, expect, it } from 'vitest'

import type { PnlBuckets } from '../api/generated/PnlBuckets'
import { buildHeadline } from './headline'

function buckets(overrides: Partial<PnlBuckets> = {}): PnlBuckets {
  return {
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
    actual: 0,
    ...overrides,
  }
}

describe('buildHeadline', () => {
  it('names the biggest gain and biggest loss, with a net figure', () => {
    const sentence = buildHeadline(buckets({ theta: 2_100, vega: -5_000 }))
    expect(sentence).toBe('Today you made $2.1k from theta and lost $5.0k to vega. Net −$2.9k.')
  })

  it('a gain-only day omits the loss clause', () => {
    const sentence = buildHeadline(buckets({ theta: 2_100, gamma: 500 }))
    expect(sentence).toBe('Today you made $2.1k from theta. Net +$2.6k.')
  })

  it('a loss-only day omits the gain clause', () => {
    const sentence = buildHeadline(buckets({ vega: -5_000, theta: -100 }))
    expect(sentence).toBe('Today you lost $5.0k to vega. Net −$5.1k.')
  })

  it('an all-zero day gets its own flat sentence, naming no bucket', () => {
    const sentence = buildHeadline(buckets())
    expect(sentence).toBe(
      "Today's P&L was flat across delta, gamma, vega, theta, rho, higher-order effects and dividends. Net $0.00.",
    )
  })

  it('names dividends when cash is the biggest gain', () => {
    const sentence = buildHeadline(buckets({ delta: -100, dividends: 100 }))
    expect(sentence).toBe('Today you made $100 from dividends and lost $100 to delta. Net $0.00.')
  })

  it('ignores residual and actual entirely, even when they dwarf the named buckets', () => {
    const sentence = buildHeadline(buckets({ theta: 10, residual: 1_000_000, actual: 1_000_010 }))
    expect(sentence).toBe('Today you made $10 from theta. Net +$10.')
  })

  it('breaks a tied gain by fixed bucket order (delta before gamma)', () => {
    const sentence = buildHeadline(buckets({ delta: 100, gamma: 100 }))
    expect(sentence).toContain('from delta')
    expect(sentence).not.toContain('from gamma')
  })

  it('breaks a tied loss by fixed bucket order (vega before theta)', () => {
    const sentence = buildHeadline(buckets({ vega: -100, theta: -100 }))
    expect(sentence).toContain('to vega')
    expect(sentence).not.toContain('to theta')
  })

  it('a net of exactly zero from offsetting buckets still names both movers', () => {
    const sentence = buildHeadline(buckets({ theta: 500, vega: -500 }))
    expect(sentence).toBe('Today you made $500 from theta and lost $500 to vega. Net $0.00.')
  })
})
