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
