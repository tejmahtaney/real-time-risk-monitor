import { describe, expect, it } from 'vitest'

import type { PositionSpec } from '../api/generated/PositionSpec'
import { position } from '../test/fixtures'
import { diffBooks, entryFromRisk, entryFromSpec, type BookEntry } from './portfolioDiff'

const entry = (id: string, quantity: number): BookEntry => ({ id, description: id.toUpperCase(), quantity })

describe('diffBooks', () => {
  it('finds added, removed and changed positions', () => {
    const current = [entry('a', 1), entry('b', 2), entry('c', 3)]
    const target = [entry('b', 2), entry('c', -3), entry('d', 4)]
    const diff = diffBooks(current, target)
    expect(diff.added.map((e) => e.id)).toEqual(['d'])
    expect(diff.removed.map((e) => e.id)).toEqual(['a'])
    expect(diff.changed).toEqual([{ id: 'c', description: 'C', from: 3, to: -3 }])
    expect(diff.identical).toBe(false)
  })

  it('reports identical books as identical', () => {
    const book = [entry('a', 1), entry('b', 2)]
    const diff = diffBooks(book, [...book].reverse())
    expect(diff).toEqual({ added: [], removed: [], changed: [], identical: true })
  })

  it('treats two empty books as identical', () => {
    expect(diffBooks([], []).identical).toBe(true)
  })
})

describe('entries from the two shapes', () => {
  it('reads a live position', () => {
    expect(entryFromRisk(position({ id: 'x', quantity: -2 }))).toEqual({
      id: 'x',
      description: 'AAPL 2026-10-16 C 230',
      quantity: -2,
    })
  })

  it('describes an option spec and an equity spec', () => {
    const base: PositionSpec = {
      id: 's1',
      underlying: 'AAPL',
      kind: 'call',
      strike: '230',
      expiry: '2026-10-16',
      quantity: 5,
      entry_price: null,
      iv: null,
    }
    expect(entryFromSpec(base).description).toBe('AAPL 2026-10-16 C 230')
    const equity = entryFromSpec({ ...base, kind: 'equity', strike: null, expiry: null })
    expect(equity.description).toBe('AAPL shares')
    expect(equity.quantity).toBe(5)
  })
})
