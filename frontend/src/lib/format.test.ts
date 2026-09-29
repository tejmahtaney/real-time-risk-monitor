import { describe, expect, it } from 'vitest'

import {
  formatCompactCurrency,
  formatCompactNumber,
  formatPercent,
  formatPerDay,
  formatPerVolPoint,
  signColor,
} from './format'

describe('formatCompactCurrency', () => {
  it('formats thousands with a k suffix', () => {
    expect(formatCompactCurrency(2_100)).toBe('$2.1k')
  })

  it('formats negative values with a real minus sign, not a hyphen', () => {
    expect(formatCompactCurrency(-5_000)).toBe('−$5.0k')
  })

  it('formats millions and billions', () => {
    expect(formatCompactCurrency(1_500_000)).toBe('$1.5M')
    expect(formatCompactCurrency(-2_300_000_000)).toBe('−$2.3B')
  })

  it('treats negative zero as plain zero, with no stray minus sign', () => {
    expect(formatCompactCurrency(-0)).toBe('$0.00')
  })

  it('handles a small sub-thousand value without a suffix', () => {
    expect(formatCompactCurrency(42.5)).toBe('$43')
    expect(formatCompactCurrency(4.25)).toBe('$4.25')
  })

  it('handles a very small non-zero value without collapsing to zero', () => {
    expect(formatCompactCurrency(0.0004)).toBe('$0.00040')
  })

  it('handles a very large value with the largest available suffix', () => {
    expect(formatCompactCurrency(3_200_000_000_000)).toBe('$3.2T')
  })

  it('renders non-finite values as an em dash rather than throwing or printing NaN', () => {
    expect(formatCompactCurrency(Number.NaN)).toBe('—')
    expect(formatCompactCurrency(Number.POSITIVE_INFINITY)).toBe('—')
  })
})

describe('formatCompactNumber', () => {
  it('formats without a currency sign', () => {
    expect(formatCompactNumber(1_200)).toBe('1.2k')
    expect(formatCompactNumber(-300)).toBe('−300')
  })
})

describe('signColor', () => {
  it('is neutral for zero and negative zero', () => {
    expect(signColor(0)).toBe('text-neutral')
    expect(signColor(-0)).toBe('text-neutral')
  })

  it('is up for positive and down for negative', () => {
    expect(signColor(1)).toBe('text-up')
    expect(signColor(-1)).toBe('text-down')
  })

  it('is neutral for a non-finite value', () => {
    expect(signColor(Number.NaN)).toBe('text-neutral')
  })
})

describe('formatPercent', () => {
  it('formats a fraction as a percentage', () => {
    expect(formatPercent(0.5)).toBe('50.0%')
    expect(formatPercent(1.02)).toBe('102.0%')
  })

  it('honours a custom digit count', () => {
    expect(formatPercent(0.12345, 2)).toBe('12.35%')
  })
})

describe('unit labels', () => {
  it('labels a vega figure per vol point', () => {
    expect(formatPerVolPoint(1_200)).toBe('$1.2k / vol pt')
  })

  it('labels a theta figure per day', () => {
    expect(formatPerDay(-450)).toBe('−$450 / day')
  })
})
