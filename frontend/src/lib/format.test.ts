import { afterEach, describe, expect, it, vi } from 'vitest'

import { formatCompactCurrency, formatCompactNumber, formatPercent, formatSimTime, signColor } from './format'

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

describe('formatSimTime', () => {
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it('formats an ISO timestamp as an ET clock with seconds by default', () => {
    // 14:16:03 UTC = 10:16:03 EDT (America/New_York is UTC-4 in September, under DST).
    expect(formatSimTime('2026-09-29T14:16:03Z')).toBe('10:16:03 ET')
  })

  it("renders in America/New_York regardless of the browser's own local timezone", () => {
    // Asia/Singapore (UTC+8) is a fixed offset with no DST, chosen specifically to
    // contrast with US Eastern: the same instant must print the same ET wall clock
    // no matter what this line sets TZ to.
    vi.stubEnv('TZ', 'Asia/Singapore')
    expect(formatSimTime('2026-09-29T14:16:03Z')).toBe('10:16:03 ET')
  })

  it('drops seconds when asked, for a compact axis tick', () => {
    expect(formatSimTime('2026-09-29T14:16:03Z', { seconds: false })).toBe('10:16 ET')
  })

  it('adds the month and day when asked', () => {
    expect(formatSimTime('2026-09-29T14:16:03Z', { withDate: true })).toBe('Sep 29, 10:16:03 ET')
  })

  it('returns an unparseable value unchanged rather than throwing', () => {
    expect(formatSimTime('not-a-date')).toBe('not-a-date')
  })
})
