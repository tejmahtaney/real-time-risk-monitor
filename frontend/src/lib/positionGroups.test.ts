import { describe, expect, it } from 'vitest'

import { position } from '../test/fixtures'
import { groupPositions, sortRows } from './positionGroups'

describe('groupPositions', () => {
  it('groups by underlying, in symbol order', () => {
    const groups = groupPositions([
      position({ id: 'a', underlying: 'MSFT' }),
      position({ id: 'b', underlying: 'AAPL' }),
      position({ id: 'c', underlying: 'AAPL' }),
    ])
    expect(groups.map((g) => g.underlying)).toEqual(['AAPL', 'MSFT'])
    expect(groups[0]?.rows.map((r) => r.id)).toEqual(['b', 'c'])
  })

  it('sums dollar measures and share delta into the subtotal', () => {
    const groups = groupPositions([
      position({
        id: 'a',
        underlying: 'AAPL',
        market_value: 100,
        unrealised_pnl: 10,
        reporting: {
          share_delta: 50,
          dollar_delta: 200,
          share_gamma: 5,
          dollar_gamma_1pct: 3,
          vega: 7,
          theta: -2,
          rho: 0,
        },
      }),
      position({
        id: 'b',
        underlying: 'AAPL',
        market_value: 50,
        unrealised_pnl: -5,
        reporting: {
          share_delta: 20,
          dollar_delta: 80,
          share_gamma: 1,
          dollar_gamma_1pct: 1,
          vega: 2,
          theta: -1,
          rho: 0,
        },
      }),
    ])
    expect(groups).toHaveLength(1)
    expect(groups[0]?.subtotal).toEqual({
      market_value: 150,
      unrealised_pnl: 5,
      dollar_delta: 280,
      dollar_gamma_1pct: 4,
      vega: 9,
      theta: -3,
      share_delta: 70,
    })
  })

  it('has no field for a portfolio total: subtotals never combine across underlyings', () => {
    const groups = groupPositions([
      position({ id: 'a', underlying: 'AAPL' }),
      position({ id: 'b', underlying: 'MSFT' }),
    ])
    expect(groups).toHaveLength(2)
    for (const group of groups) {
      expect(Object.keys(group.subtotal).sort()).toEqual(
        ['dollar_delta', 'dollar_gamma_1pct', 'market_value', 'share_delta', 'theta', 'unrealised_pnl', 'vega'].sort(),
      )
    }
  })
})

describe('sortRows', () => {
  const rows = [
    position({ id: 'a', quantity: 3 }),
    position({ id: 'b', quantity: 1 }),
    position({ id: 'c', quantity: 2 }),
  ]

  it('sorts ascending by a numeric column', () => {
    const sorted = sortRows(rows, { column: 'quantity', direction: 'asc' })
    expect(sorted.map((r) => r.id)).toEqual(['b', 'c', 'a'])
  })

  it('sorts descending by a numeric column', () => {
    const sorted = sortRows(rows, { column: 'quantity', direction: 'desc' })
    expect(sorted.map((r) => r.id)).toEqual(['a', 'c', 'b'])
  })

  it('does not mutate the input array', () => {
    const copy = [...rows]
    sortRows(rows, { column: 'quantity', direction: 'asc' })
    expect(rows).toEqual(copy)
  })

  it('sorts by a string column', () => {
    const named = [
      position({ id: 'a', description: 'Zeta' }),
      position({ id: 'b', description: 'Alpha' }),
    ]
    const sorted = sortRows(named, { column: 'description', direction: 'asc' })
    expect(sorted.map((r) => r.id)).toEqual(['b', 'a'])
  })
})
