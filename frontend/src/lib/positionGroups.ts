/**
 * Pure helpers for the positions table: grouping by underlying, per-group dollar
 * subtotals, and column sorting. Kept dependency-free so they are trivial to unit
 * test without mounting the table itself.
 */
import type { PositionRisk } from '../api/generated/PositionRisk'

/** A group's dollar-measure subtotal, plus share delta (meaningful for one
 * underlying). No share gamma and no portfolio total of share delta anywhere:
 * see CLAUDE.md's units section on why share measures never add across underlyings. */
export interface PositionSubtotal {
  market_value: number
  unrealised_pnl: number
  dollar_delta: number
  dollar_gamma_1pct: number
  vega: number
  theta: number
  share_delta: number
}

export interface PositionGroup {
  underlying: string
  rows: PositionRisk[]
  subtotal: PositionSubtotal
}

/** Groups positions by underlying (symbol order) and sums each group's dollar
 * measures plus share delta. Relies on the server already zeroing the figures
 * that an expired or error position must not contribute (see `PositionRisk`'s
 * doc comment), so a plain sum here is correct without re-deriving status rules. */
export function groupPositions(positions: PositionRisk[]): PositionGroup[] {
  const byUnderlying = new Map<string, PositionRisk[]>()
  for (const position of positions) {
    const rows = byUnderlying.get(position.underlying)
    if (rows) {
      rows.push(position)
    } else {
      byUnderlying.set(position.underlying, [position])
    }
  }
  return [...byUnderlying.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([underlying, rows]) => ({ underlying, rows, subtotal: subtotalOf(rows) }))
}

function subtotalOf(rows: PositionRisk[]): PositionSubtotal {
  return rows.reduce<PositionSubtotal>(
    (acc, p) => ({
      market_value: acc.market_value + p.market_value,
      unrealised_pnl: acc.unrealised_pnl + p.unrealised_pnl,
      dollar_delta: acc.dollar_delta + p.reporting.dollar_delta,
      dollar_gamma_1pct: acc.dollar_gamma_1pct + p.reporting.dollar_gamma_1pct,
      vega: acc.vega + p.reporting.vega,
      theta: acc.theta + p.reporting.theta,
      share_delta: acc.share_delta + p.reporting.share_delta,
    }),
    {
      market_value: 0,
      unrealised_pnl: 0,
      dollar_delta: 0,
      dollar_gamma_1pct: 0,
      vega: 0,
      theta: 0,
      share_delta: 0,
    },
  )
}

/** The columns a row can be sorted by. */
export type SortColumn =
  | 'description'
  | 'quantity'
  | 'spot'
  | 'mark'
  | 'market_value'
  | 'unrealised_pnl'
  | 'share_delta'
  | 'dollar_delta'
  | 'share_gamma'
  | 'vega'
  | 'theta'
  | 'status'

export type SortDirection = 'asc' | 'desc'

export interface SortState {
  column: SortColumn
  direction: SortDirection
}

function sortValue(row: PositionRisk, column: SortColumn): string | number {
  switch (column) {
    case 'description':
      return row.description
    case 'quantity':
      return row.quantity
    case 'spot':
      return row.spot
    case 'mark':
      return row.mark
    case 'market_value':
      return row.market_value
    case 'unrealised_pnl':
      return row.unrealised_pnl
    case 'share_delta':
      return row.reporting.share_delta
    case 'dollar_delta':
      return row.reporting.dollar_delta
    case 'share_gamma':
      return row.reporting.share_gamma
    case 'vega':
      return row.reporting.vega
    case 'theta':
      return row.reporting.theta
    case 'status':
      return row.status.state
  }
}

/** Sorts a copy of `rows` by the given column and direction. Stable (Array.sort
 * in every engine this project targets is stable), so equal keys keep their
 * original relative order. */
export function sortRows(rows: PositionRisk[], sort: SortState): PositionRisk[] {
  const sign = sort.direction === 'asc' ? 1 : -1
  return [...rows].sort((a, b) => {
    const av = sortValue(a, sort.column)
    const bv = sortValue(b, sort.column)
    if (typeof av === 'string' || typeof bv === 'string') {
      return sign * String(av).localeCompare(String(bv))
    }
    return sign * (av - bv)
  })
}

const SORT_STORAGE_KEY = 'rrm.positions.sort'

/** Reads the remembered sort from `localStorage`, or `null` if there is none or
 * storage is unavailable (private browsing, a disabled site setting, …). Never
 * throws: a broken read just means "no remembered sort". */
export function loadSortState(): SortState | null {
  try {
    const raw = window.localStorage.getItem(SORT_STORAGE_KEY)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      'column' in parsed &&
      'direction' in parsed
    ) {
      return parsed as SortState
    }
    return null
  } catch {
    return null
  }
}

/** Persists the sort to `localStorage`. Never throws. */
export function saveSortState(sort: SortState): void {
  try {
    window.localStorage.setItem(SORT_STORAGE_KEY, JSON.stringify(sort))
  } catch {
    // Storage unavailable or full: the sort just won't be remembered.
  }
}
