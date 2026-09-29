/**
 * Pure data transforms for the allocation donut: gross exposure shares (straight
 * from `RiskSummary::concentration`) and |vega| shares (derived here, since
 * `Concentration` only ever carries gross exposure — see its own doc comment).
 */
import type { Concentration } from '../api/generated/Concentration'
import type { Symbol } from '../api/generated/Symbol'
import type { UnderlyingRisk } from '../api/generated/UnderlyingRisk'

export interface AllocationSlice {
  symbol: string
  /** The measure's raw dollar value for this underlying (always >= 0). */
  value: number
  /** `value` ÷ the total across every underlying, 0 to 1. */
  fraction: number
}

/** The donut's two measures. */
export type AllocationMeasure = 'gross_exposure' | 'vega'

/** Gross exposure shares, straight from the server's own `concentration` (already
 * sorted largest-first, ties in symbol order — kept as given, not re-sorted). */
export function grossExposureSlices(concentration: Concentration[]): AllocationSlice[] {
  return concentration.map((c) => ({ symbol: c.symbol, value: c.gross_exposure, fraction: c.fraction }))
}

/** |vega| shares by underlying, largest first and ties in symbol order — the same
 * sorting rule `risk::aggregate`'s `concentration` uses for gross exposure, applied
 * here to vega since the server has no |vega| concentration of its own. An
 * underlying with zero total |vega| across the book gets fraction 0 for everyone,
 * matching `Concentration`'s own all-zero rule, rather than `NaN` from a 0/0 divide. */
export function vegaSlices(byUnderlying: Record<Symbol, UnderlyingRisk>): AllocationSlice[] {
  const entries = Object.entries(byUnderlying).map(([symbol, risk]) => ({
    symbol,
    value: Math.abs(risk.vega),
  }))
  const total = entries.reduce((sum, e) => sum + e.value, 0)
  return entries
    .map((e) => ({ ...e, fraction: total > 0 ? e.value / total : 0 }))
    .sort((a, b) => b.fraction - a.fraction || a.symbol.localeCompare(b.symbol))
}

/** Picks the slices for whichever measure is toggled on. */
export function allocationSlices(
  measure: AllocationMeasure,
  concentration: Concentration[],
  byUnderlying: Record<Symbol, UnderlyingRisk>,
): AllocationSlice[] {
  return measure === 'gross_exposure' ? grossExposureSlices(concentration) : vegaSlices(byUnderlying)
}

/** The centre label's total: the sum of every slice's value. */
export function allocationTotal(slices: AllocationSlice[]): number {
  return slices.reduce((sum, s) => sum + s.value, 0)
}
