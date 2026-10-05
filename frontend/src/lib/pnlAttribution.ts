/**
 * Pure data transforms for the P&L attribution chart: picking a level's buckets out
 * of a `PnlReport`, the ordered bar rows for the headline chart, and the
 * per-underlying stacked breakdown.
 */
import type { PnlBuckets } from '../api/generated/PnlBuckets'
import type { PnlLevel } from '../api/generated/PnlLevel'
import type { PnlReport } from '../api/generated/PnlReport'
import type { Symbol } from '../api/generated/Symbol'

/** The attribution chart's period toggle. */
export type PnlPeriod = 'today' | 'since_start' | 'previous_day'

export const PNL_PERIODS: PnlPeriod[] = ['today', 'since_start', 'previous_day']

/** Every `PnlBuckets` field, in the order the horizontal bar chart draws them. */
export const BUCKET_ORDER: Array<{ key: keyof PnlBuckets; label: string }> = [
  { key: 'delta', label: 'Delta' },
  { key: 'gamma', label: 'Gamma' },
  { key: 'vega', label: 'Vega' },
  { key: 'theta', label: 'Theta' },
  { key: 'rho', label: 'Rho' },
  { key: 'higher_order', label: 'Higher-order' },
  { key: 'dividends', label: 'Dividends' },
  { key: 'residual', label: 'Residual' },
  { key: 'actual', label: 'Actual' },
]

/** The buckets stacked in the per-underlying breakdown: the Taylor decomposition
 * plus the residual. `actual` is deliberately left out — `explained() + residual`
 * already equals `actual` (see `PnlBuckets`'s own doc comment), so stacking it too
 * would double the visible total rather than showing its composition. */
export const STACK_KEYS: Array<keyof PnlBuckets> = [
  'delta',
  'gamma',
  'vega',
  'theta',
  'rho',
  'higher_order',
  'dividends',
  'residual',
]

export interface BucketRow {
  key: keyof PnlBuckets
  label: string
  value: number
}

/** The bar chart's rows for one set of buckets, in `BUCKET_ORDER`. */
export function bucketRows(buckets: PnlBuckets): BucketRow[] {
  return BUCKET_ORDER.map(({ key, label }) => ({ key, label, value: buckets[key] }))
}

/** Picks `period`'s portfolio buckets out of a `PnlReport`. `null` for `'previous_day'`
 * before the first session roll (`PnlReport.previous_day` starts `null`). */
export function portfolioBucketsForPeriod(report: PnlReport, period: PnlPeriod): PnlBuckets | null {
  if (period === 'previous_day') return report.previous_day?.portfolio ?? null
  return report.portfolio[period]
}

export interface UnderlyingStack {
  symbol: string
  values: Partial<Record<keyof PnlBuckets, number>>
}

function levelBuckets(level: PnlLevel, period: Exclude<PnlPeriod, 'previous_day'>): PnlBuckets {
  return level[period]
}

/** The per-underlying stacked bar's rows for `period`, symbols in alphabetical order
 * so the stack's order does not depend on iteration order of a plain object. */
export function underlyingStacks(report: PnlReport, period: PnlPeriod): UnderlyingStack[] {
  const byUnderlying: Record<Symbol, PnlBuckets> | undefined =
    period === 'previous_day'
      ? report.previous_day?.by_underlying
      : mapValues(report.by_underlying, (level) => levelBuckets(level, period))
  if (!byUnderlying) return []
  return Object.entries(byUnderlying)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([symbol, buckets]) => {
      const values: Partial<Record<keyof PnlBuckets, number>> = {}
      for (const key of STACK_KEYS) values[key] = buckets[key]
      return { symbol, values }
    })
}

function mapValues<T, U>(record: Record<string, T>, fn: (value: T) => U): Record<string, U> {
  const result: Record<string, U> = {}
  for (const [key, value] of Object.entries(record)) result[key] = fn(value)
  return result
}

/** Pixels per bucket row in the attribution chart, enough for one label each. */
const BUCKET_ROW_HEIGHT = 28
/** Space for the chart's value axis and margins. */
const BUCKET_CHART_PADDING = 40

/** Chart height that gives every bucket its own labelled row (nine buckets now). */
export function bucketChartHeight(rowCount: number): number {
  return Math.max(220, rowCount * BUCKET_ROW_HEIGHT + BUCKET_CHART_PADDING)
}
