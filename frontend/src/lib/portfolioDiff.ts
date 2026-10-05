/**
 * The difference between the current book and a saved version. The two arrive in different
 * shapes (`PositionRisk` from `/api/positions`, `PositionSpec` from a version), so both are
 * reduced to a small common shape and matched on position id.
 */
import type { PositionRisk } from '../api/generated/PositionRisk'
import type { PositionSpec } from '../api/generated/PositionSpec'

/** What the diff needs to know about a position. */
export interface BookEntry {
  id: string
  description: string
  quantity: number
}

/** A position whose quantity differs between the two books. */
export interface QuantityChange {
  id: string
  description: string
  from: number
  to: number
}

/** The result of {@link diffBooks}: what loading `target` over `current` would do. */
export interface BookDiff {
  /** In the target only. */
  added: BookEntry[]
  /** In the current book only. */
  removed: BookEntry[]
  /** In both with a different quantity. */
  changed: QuantityChange[]
  /** True when loading would change nothing. */
  identical: boolean
}

/** A live position as a {@link BookEntry}. */
export function entryFromRisk(position: PositionRisk): BookEntry {
  return { id: position.id, description: position.description, quantity: position.quantity }
}

/** A saved row as a {@link BookEntry}, with a description built from its terms. */
export function entryFromSpec(spec: PositionSpec): BookEntry {
  const terms =
    spec.kind === 'equity'
      ? [spec.underlying, 'shares']
      : [spec.underlying, spec.expiry, spec.kind === 'call' ? 'C' : 'P', spec.strike]
  return {
    id: spec.id,
    description: terms.filter((part) => part !== null).join(' '),
    quantity: spec.quantity,
  }
}

/** What loading `target` over `current` would add, remove and change. Matching is by id; a
 * quantity counts as changed when it differs by more than rounding noise. */
export function diffBooks(current: BookEntry[], target: BookEntry[]): BookDiff {
  const currentById = new Map(current.map((entry) => [entry.id, entry]))
  const targetById = new Map(target.map((entry) => [entry.id, entry]))
  const added = target.filter((entry) => !currentById.has(entry.id))
  const removed = current.filter((entry) => !targetById.has(entry.id))
  const changed: QuantityChange[] = []
  for (const entry of target) {
    const before = currentById.get(entry.id)
    if (before !== undefined && Math.abs(before.quantity - entry.quantity) > 1e-9) {
      changed.push({ id: entry.id, description: entry.description, from: before.quantity, to: entry.quantity })
    }
  }
  return {
    added,
    removed,
    changed,
    identical: added.length === 0 && removed.length === 0 && changed.length === 0,
  }
}
