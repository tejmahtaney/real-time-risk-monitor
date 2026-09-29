/**
 * Pure data transforms for the Greeks-over-time charts: the 5 minute / 15 minute /
 * all window selector, and a render point cap so a long "all" window never asks
 * Recharts to lay out more points than the page can draw smoothly.
 */
import type { HistoryPoint } from '../api/generated/HistoryPoint'

export type ChartWindow = '5m' | '15m' | 'all'

export const CHART_WINDOWS: ChartWindow[] = ['5m', '15m', 'all']

const WINDOW_MS: Record<Exclude<ChartWindow, 'all'>, number> = {
  '5m': 5 * 60 * 1000,
  '15m': 15 * 60 * 1000,
}

/** The most points ever handed to one chart's `<Line>`/`<Bar>`. `history` is already
 * capped at 2,400 points by the store (see `HISTORY_CAP` in `riskStore.ts`); this is
 * a second, tighter cap so choosing "all" over a long session does not make a chart
 * lay out thousands of points. Keeps the most recent points, since a chart reads
 * left-to-right toward "now". */
export const MAX_CHART_POINTS = 600

/** Filters `history` to the window ending at its own last point's `sim_time` (not
 * wall-clock time), since simulated time can run faster or slower than real time.
 * An empty or unparseable `sim_time` on the last point leaves `history` untouched
 * for `'5m'`/`'15m'` (nothing to measure the window from), and `'all'` always
 * returns everything, per its name. */
export function filterWindow(history: HistoryPoint[], window: ChartWindow): HistoryPoint[] {
  if (window === 'all' || history.length === 0) return history
  const lastPoint = history[history.length - 1]
  const last = lastPoint === undefined ? Number.NaN : Date.parse(lastPoint.sim_time)
  if (Number.isNaN(last)) return history
  const cutoff = last - WINDOW_MS[window]
  return history.filter((point) => {
    const t = Date.parse(point.sim_time)
    return !Number.isNaN(t) && t >= cutoff
  })
}

/** Keeps at most the most recent `maxPoints` of `points`. A no-op once a series is
 * already within the cap. */
export function capHistory(points: HistoryPoint[], maxPoints = MAX_CHART_POINTS): HistoryPoint[] {
  return points.length > maxPoints ? points.slice(points.length - maxPoints) : points
}
