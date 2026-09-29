import { describe, expect, it } from 'vitest'

import { symbolColor } from './symbolColors'

describe('symbolColor', () => {
  it('gives the five calibrated symbols distinct, fixed colours', () => {
    const symbols = ['SPY', 'AAPL', 'MSFT', 'NVDA', 'TSLA']
    const colours = symbols.map(symbolColor)
    expect(new Set(colours).size).toBe(symbols.length)
  })

  it('is stable across repeated calls', () => {
    expect(symbolColor('SPY')).toBe(symbolColor('SPY'))
    expect(symbolColor('GOOG')).toBe(symbolColor('GOOG'))
  })

  it('is unaffected by what other symbols exist', () => {
    const alone = symbolColor('SPY')
    // Colouring SPY does not depend on having looked at AAPL, MSFT, etc. first.
    void symbolColor('AAPL')
    void symbolColor('MSFT')
    expect(symbolColor('SPY')).toBe(alone)
  })

  it('gives an unknown symbol a real palette colour, not a crash or blank', () => {
    const colour = symbolColor('GOOG')
    expect(colour).toMatch(/^#[0-9a-f]{6}$/)
  })

  it('never gives an unknown symbol one of the five known symbols colours', () => {
    const known = new Set(['SPY', 'AAPL', 'MSFT', 'NVDA', 'TSLA'].map(symbolColor))
    expect(known.has(symbolColor('GOOG'))).toBe(false)
    expect(known.has(symbolColor('QQQ'))).toBe(false)
  })
})
