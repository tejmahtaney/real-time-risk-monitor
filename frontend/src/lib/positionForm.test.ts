import { describe, expect, it } from 'vitest'

import { buildPositionSpec, validateAddPositionForm, type AddPositionFormValues } from './positionForm'

function values(overrides: Partial<AddPositionFormValues> = {}): AddPositionFormValues {
  return {
    underlying: 'AAPL',
    kind: 'call',
    strike: 'ATM+5%',
    expiry: '+30d',
    quantity: '2',
    ...overrides,
  }
}

describe('validateAddPositionForm', () => {
  it('accepts a well-formed option row', () => {
    expect(validateAddPositionForm(values())).toEqual({})
  })

  it('accepts an equity row with blank strike and expiry', () => {
    expect(
      validateAddPositionForm(values({ kind: 'equity', strike: '', expiry: '', quantity: '100' })),
    ).toEqual({})
  })

  it('accepts an absolute strike and an absolute expiry', () => {
    expect(validateAddPositionForm(values({ strike: '230', expiry: '2026-10-16' }))).toEqual({})
  })

  it('accepts a bare ATM strike', () => {
    expect(validateAddPositionForm(values({ strike: 'ATM' }))).toEqual({})
  })

  it('rejects a malformed underlying', () => {
    expect(validateAddPositionForm(values({ underlying: 'TOO LONG SYMBOL' }))).toHaveProperty('underlying')
  })

  it('requires strike and expiry for an option', () => {
    const errors = validateAddPositionForm(values({ strike: '', expiry: '' }))
    expect(errors).toHaveProperty('strike')
    expect(errors).toHaveProperty('expiry')
  })

  it('rejects a strike offset at or below -100%', () => {
    expect(validateAddPositionForm(values({ strike: 'ATM-100%' }))).toHaveProperty('strike')
  })

  it('rejects a malformed strike', () => {
    for (const bad of ['abc', '0', '-5', 'ATM5%', 'ATM+x%']) {
      expect(validateAddPositionForm(values({ strike: bad }))).toHaveProperty('strike')
    }
  })

  it('rejects a malformed expiry', () => {
    for (const bad of ['tomorrow', '+d', '+3.5d', '16/10/2026']) {
      expect(validateAddPositionForm(values({ expiry: bad }))).toHaveProperty('expiry')
    }
  })

  it('rejects a relative expiry beyond the accepted range', () => {
    expect(validateAddPositionForm(values({ expiry: '+3651d' }))).toHaveProperty('expiry')
  })

  it('rejects a blank or zero quantity', () => {
    expect(validateAddPositionForm(values({ quantity: '' }))).toHaveProperty('quantity')
    expect(validateAddPositionForm(values({ quantity: '0' }))).toHaveProperty('quantity')
    expect(validateAddPositionForm(values({ quantity: 'abc' }))).toHaveProperty('quantity')
  })

  it('accepts a negative quantity (a short position)', () => {
    expect(validateAddPositionForm(values({ quantity: '-3' }))).toEqual({})
  })
})

describe('buildPositionSpec', () => {
  it('nulls out strike and expiry for equity', () => {
    const spec = buildPositionSpec(values({ kind: 'equity', strike: '', expiry: '', quantity: '10' }))
    expect(spec.strike).toBeNull()
    expect(spec.expiry).toBeNull()
    expect(spec.kind).toBe('equity')
    expect(spec.quantity).toBe(10)
  })

  it('carries the strike and expiry text through for an option', () => {
    const spec = buildPositionSpec(values())
    expect(spec.strike).toBe('ATM+5%')
    expect(spec.expiry).toBe('+30d')
    expect(spec.underlying).toBe('AAPL')
  })

  it('generates a unique id', () => {
    const a = buildPositionSpec(values())
    const b = buildPositionSpec(values())
    expect(a.id).not.toBe(b.id)
  })
})
