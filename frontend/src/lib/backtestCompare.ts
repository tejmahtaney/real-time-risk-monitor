/** Two saved backtest runs on one axis. */
import type { DailyRow } from '../api/generated/DailyRow'

/** One date of the overlay: each run's cumulative P&L, `null` where a run has no such date. */
export interface OverlayPoint {
  date: string
  a: number | null
  b: number | null
}

/** Cumulative P&L of two runs by date. Dates are unioned and sorted, so runs over different
 * periods still share one axis. */
export function overlaySeries(a: DailyRow[], b: DailyRow[]): OverlayPoint[] {
  const byA = new Map(a.map((row) => [row.date, row.cumulative_pnl]))
  const byB = new Map(b.map((row) => [row.date, row.cumulative_pnl]))
  const dates = [...new Set([...byA.keys(), ...byB.keys()])].sort()
  return dates.map((date) => ({ date, a: byA.get(date) ?? null, b: byB.get(date) ?? null }))
}
