/**
 * Pure data transforms for the backtest results charts, built only from a run's `DailyRow`s
 * (`GET /results`), never from the stream, so a run looks the same after a server restart.
 */
import type { DailyRow } from '../api/generated/DailyRow'

/** The P&L buckets stacked under the cumulative line, with their chart colours. */
export const BUCKET_SERIES = [
  { key: 'delta', label: 'Delta', color: '#60a5fa' },
  { key: 'gamma', label: 'Gamma', color: '#34d399' },
  { key: 'vega', label: 'Vega', color: '#a78bfa' },
  { key: 'theta', label: 'Theta', color: '#fbbf24' },
  { key: 'rho', label: 'Rho', color: '#22d3ee' },
  { key: 'higher_order', label: 'Higher-order', color: '#f472b6' },
  { key: 'dividends', label: 'Dividends', color: '#fb923c' },
  { key: 'residual', label: 'Residual', color: '#9ca3af' },
] as const

/** A bucket's key in {@link BUCKET_SERIES}. */
export type BucketKey = (typeof BUCKET_SERIES)[number]['key']

/** One date of the cumulative chart: the running total and the running total of each bucket. */
export type CumulativePoint = { date: string; total: number } & Record<BucketKey, number>

/** Running totals by bucket, and the cumulative actual P&L, one point per date. */
export function cumulativeSeries(rows: DailyRow[]): CumulativePoint[] {
  const sums: Record<BucketKey, number> = {
    delta: 0,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
  }
  return rows.map((row) => {
    for (const { key } of BUCKET_SERIES) sums[key] += row[key]
    return { date: row.date, total: row.cumulative_pnl, ...sums }
  })
}

/** The data key of a bucket's positive part in a {@link StackedPoint}. */
export function posKey(key: BucketKey): `${BucketKey}_pos` {
  return `${key}_pos`
}

/** The data key of a bucket's negative part in a {@link StackedPoint}. */
export function negKey(key: BucketKey): `${BucketKey}_neg` {
  return `${key}_neg`
}

/** A {@link CumulativePoint} with each bucket split into its positive and negative part. */
export type StackedPoint = CumulativePoint & Record<`${BucketKey}_pos` | `${BucketKey}_neg`, number>

/** Cumulative points ready for sign-aware stacking: positive buckets stack upwards from zero in
 * one stack and negative buckets stack downwards in another, so offsetting buckets do not
 * inflate the chart beyond the data. */
export function stackedSeries(rows: DailyRow[]): StackedPoint[] {
  return cumulativeSeries(rows).map((point) => {
    const parts: Record<string, number> = {}
    for (const { key } of BUCKET_SERIES) {
      parts[posKey(key)] = Math.max(point[key], 0)
      parts[negKey(key)] = Math.min(point[key], 0)
    }
    return { ...point, ...parts } as StackedPoint
  })
}

/** The Y axis range `[min, max]` that fits the positive stack, the negative stack and the
 * cumulative line, with 5 % headroom, always including zero. */
export function stackedDomain(points: StackedPoint[]): [number, number] {
  let lo = 0
  let hi = 0
  for (const point of points) {
    let up = 0
    let down = 0
    for (const { key } of BUCKET_SERIES) {
      up += point[posKey(key)]
      down += point[negKey(key)]
    }
    hi = Math.max(hi, up, point.total)
    lo = Math.min(lo, down, point.total)
  }
  const pad = (hi - lo) * 0.05
  return [lo === 0 ? 0 : lo - pad, hi === 0 ? 0 : hi + pad]
}

/** One date of the drawdown chart. `drawdown` is zero or negative: how far the cumulative P&L
 * is below its running peak, with the start counted as a peak of zero. */
export interface DrawdownPoint {
  date: string
  drawdown: number
}

/** The drawdown below the running peak, one point per date. */
export function drawdownSeries(rows: DailyRow[]): DrawdownPoint[] {
  let peak = 0
  return rows.map((row) => {
    peak = Math.max(peak, row.cumulative_pnl)
    return { date: row.date, drawdown: row.cumulative_pnl - peak }
  })
}
