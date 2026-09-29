/**
 * Client-side validation for the "Add position" form, mirroring the formats
 * `portfolio::spec::StrikeSpec` and `ExpirySpec` parse on the server
 * (`crates/portfolio/src/spec.rs`), so an obviously malformed entry is caught
 * before a round trip. The server remains the source of truth: a spec that
 * passes here can still be rejected server-side (a bad underlying, a strike
 * that resolves to zero, …), and that error is shown inline too.
 */
import type { InstrumentKind } from '../api/generated/InstrumentKind'
import type { PositionSpec } from '../api/generated/PositionSpec'

export interface AddPositionFormValues {
  underlying: string
  kind: InstrumentKind
  strike: string
  expiry: string
  quantity: string
}

export type AddPositionFieldErrors = Partial<Record<keyof AddPositionFormValues, string>>

const SYMBOL_RE = /^[A-Za-z0-9.-]{1,10}$/
const ABSOLUTE_STRIKE_RE = /^\d+(\.\d+)?$/
const ATM_STRIKE_RE = /^ATM(?<sign>[+-])(?<pct>\d+(\.\d+)?)%$/i
const RELATIVE_EXPIRY_RE = /^\+(?<days>\d{1,4})[dD]$/
const ABSOLUTE_EXPIRY_RE = /^\d{4}-\d{2}-\d{2}$/
const MAX_RELATIVE_DAYS = 3650

function validateUnderlying(value: string): string | undefined {
  if (!SYMBOL_RE.test(value.trim())) {
    return 'expected 1-10 characters of letters, digits, "." or "-"'
  }
  return undefined
}

function validateStrike(value: string, kind: InstrumentKind): string | undefined {
  const text = value.trim()
  if (kind === 'equity') return undefined
  if (text === '') return 'strike is required for an option'
  if (text.toUpperCase() === 'ATM') return undefined
  if (ABSOLUTE_STRIKE_RE.test(text)) {
    return Number(text) > 0 ? undefined : 'strike must be a positive number'
  }
  const atm = ATM_STRIKE_RE.exec(text)
  if (atm?.groups) {
    const pct = Number(atm.groups.pct) * (atm.groups.sign === '-' ? -1 : 1)
    return pct > -100 ? undefined : 'strike offset must be above -100%'
  }
  return 'expected a number, ATM, ATM+N% or ATM-N%'
}

function validateExpiry(value: string, kind: InstrumentKind): string | undefined {
  const text = value.trim()
  if (kind === 'equity') return undefined
  if (text === '') return 'expiry is required for an option'
  if (ABSOLUTE_EXPIRY_RE.test(text)) return undefined
  const relative = RELATIVE_EXPIRY_RE.exec(text)
  if (relative?.groups) {
    return Number(relative.groups.days) <= MAX_RELATIVE_DAYS
      ? undefined
      : `at most +${MAX_RELATIVE_DAYS}d is allowed`
  }
  return 'expected YYYY-MM-DD, +0d or +Nd'
}

function validateQuantity(value: string): string | undefined {
  const text = value.trim()
  if (text === '') return 'quantity is required'
  const n = Number(text)
  if (!Number.isFinite(n)) return 'expected a number'
  if (n === 0) return 'quantity must not be zero'
  return undefined
}

/** Validates every field. Returns an empty object when the form is ready to submit. */
export function validateAddPositionForm(values: AddPositionFormValues): AddPositionFieldErrors {
  const errors: AddPositionFieldErrors = {}
  const underlying = validateUnderlying(values.underlying)
  if (underlying) errors.underlying = underlying
  const strike = validateStrike(values.strike, values.kind)
  if (strike) errors.strike = strike
  const expiry = validateExpiry(values.expiry, values.kind)
  if (expiry) errors.expiry = expiry
  const quantity = validateQuantity(values.quantity)
  if (quantity) errors.quantity = quantity
  return errors
}

/** Builds the `PositionSpec` to submit from already-validated form values. A
 * fresh id is generated client-side since the form has no id field of its own. */
export function buildPositionSpec(values: AddPositionFormValues): PositionSpec {
  const isOption = values.kind !== 'equity'
  const idSuffix =
    typeof crypto !== 'undefined' && 'randomUUID' in crypto
      ? crypto.randomUUID().slice(0, 8)
      : Math.random().toString(36).slice(2, 10)
  return {
    id: `${values.underlying.trim().toLowerCase()}-${values.kind}-${idSuffix}`,
    underlying: values.underlying.trim().toUpperCase(),
    kind: values.kind,
    strike: isOption ? values.strike.trim() : null,
    expiry: isOption ? values.expiry.trim() : null,
    quantity: Number(values.quantity),
    entry_price: null,
    iv: null,
  }
}
