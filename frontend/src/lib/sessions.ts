/** Pure helpers for the History page: list formatting, date ranges, events and comparison. */
import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { SessionEvent } from '../api/generated/SessionEvent'
import type { SessionSummary } from '../api/generated/SessionSummary'
import { formatCompactCurrency, formatSimTime } from './format'
import { formatBytes } from './usage'

/** One session as the table shows it. */
export interface SessionRowView {
  id: string
  start: string
  /** The end time, or `cut short` when the session never recorded one. */
  end: string
  cutShort: boolean
  days: number
  size: string
  commit: string
  /** The total P&L, or `—` when none was recorded. */
  pnl: string
  pnlValue: number | null
}

/** A session's start or end as a New York date and time. These are real wall-clock instants. */
function formatWall(iso: string): string {
  return formatSimTime(iso, { seconds: false, withDate: true })
}

/** The table row for a session. `end` and `total_pnl` are null for a cut-short session. */
export function sessionRowView(session: SessionSummary): SessionRowView {
  return {
    id: session.id,
    start: formatWall(session.start),
    end: session.end === null ? 'cut short' : formatWall(session.end),
    cutShort: session.end === null,
    days: session.simulated_days,
    size: formatBytes(session.size_bytes),
    commit: session.git_commit.slice(0, 7),
    pnl: session.total_pnl === null ? '—' : formatCompactCurrency(session.total_pnl),
    pnlValue: session.total_pnl,
  }
}

/** Sessions newest first, by start time. */
export function newestFirst(sessions: SessionSummary[]): SessionSummary[] {
  return [...sessions].sort((a, b) => b.start.localeCompare(a.start))
}

const NY_DATE = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'America/New_York',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
})

/** The New York calendar date (`YYYY-MM-DD`) of an instant. */
export function marketDateOf(iso: string): string {
  return NY_DATE.format(new Date(iso))
}

/** A simulated date in a session, with the first and last history instants on it. */
export interface SessionDate {
  date: string
  first: string
  last: string
}

/** The New York dates a history covers, with each date's first and last instant. This is how
 * the range control turns a chosen date into `from` and `to` bounds without any time-zone
 * arithmetic of its own. */
export function sessionDates(points: HistoryPoint[]): SessionDate[] {
  const byDate = new Map<string, SessionDate>()
  for (const point of points) {
    const date = marketDateOf(point.sim_time)
    const found = byDate.get(date)
    if (found === undefined) byDate.set(date, { date, first: point.sim_time, last: point.sim_time })
    else if (point.sim_time > found.last) found.last = point.sim_time
    else if (point.sim_time < found.first) found.first = point.sim_time
  }
  return [...byDate.values()].sort((a, b) => a.date.localeCompare(b.date))
}

/** The `from` and `to` bounds covering `fromDate` to `toDate` inclusive, or `null` when either is
 * not a date of the session or they are the wrong way round. */
export function rangeBounds(
  dates: SessionDate[],
  fromDate: string,
  toDate: string,
): { from: string; to: string } | null {
  const from = dates.find((d) => d.date === fromDate)
  const to = dates.find((d) => d.date === toDate)
  if (from === undefined || to === undefined || from.date > to.date) return null
  return { from: from.first, to: to.last }
}

/** Event types the server tags events with, for the filter. */
export const EVENT_TYPES: ReadonlyArray<SessionEvent['type']> = [
  'alert',
  'settlement',
  'position_change',
  'portfolio_load',
  'corporate_action',
  'control',
]

/** The time an event happened, in simulated time where it has one. */
export function eventTime(event: SessionEvent): string {
  switch (event.type) {
    case 'settlement':
      return event.settled_at
    default:
      return event.sim_time
  }
}

/** One line of plain text for an event. */
export function describeEvent(event: SessionEvent): string {
  switch (event.type) {
    case 'alert':
      return `${event.name} ${event.from} → ${event.to} (${event.value.toPrecision(4)} against ${event.limit.toPrecision(4)})`
    case 'settlement':
      return `${event.description} settled at ${event.spot.toFixed(2)}, realised ${formatCompactCurrency(event.realised_pnl)}`
    case 'position_change':
      return `Position ${event.position_id} ${event.kind}`
    case 'portfolio_load':
      return `Loaded ${event.source} (${event.position_count} positions)`
    case 'corporate_action':
      return event.message
    case 'control':
      return event.action.action === 'set_speed'
        ? `Speed set to ${event.action.speed}×`
        : event.action.action === 'pause'
          ? 'Paused'
          : 'Resumed'
  }
}

/** The Greeks the comparison can plot. */
export type CompareMetric = 'dollar_delta' | 'dollar_gamma_1pct' | 'vega' | 'theta' | 'var_99'

/** Cumulative P&L at each history point. `pnl_today` restarts at each simulated day, so the
 * total carries the last value of every earlier New York date. */
export function cumulativePnl(points: HistoryPoint[]): number[] {
  const out: number[] = []
  let carried = 0
  let day: string | null = null
  let lastToday = 0
  for (const point of points) {
    const date = marketDateOf(point.sim_time)
    if (day !== null && date !== day) carried += lastToday
    day = date
    lastToday = point.pnl_today.actual
    out.push(carried + lastToday)
  }
  return out
}

/** One x position of the comparison: hours of simulated time since each session's own first
 * point, with each session's value there (`null` where it has no point). */
export interface ComparePoint {
  hours: number
  pnlA: number | null
  pnlB: number | null
  greekA: number | null
  greekB: number | null
}

const MS_PER_HOUR = 3_600_000

/** Two sessions on one axis, aligned by simulated time elapsed since each session's first
 * history point. Every session starts on the same simulated date, so wall time would not
 * line them up. Each session keeps its own sample times; a gap in the other is `null`. */
export function alignSessions(
  a: HistoryPoint[],
  b: HistoryPoint[],
  metric: CompareMetric,
): ComparePoint[] {
  const side = (points: HistoryPoint[]): Map<number, { pnl: number; greek: number | null }> => {
    const out = new Map<number, { pnl: number; greek: number | null }>()
    const first = points[0]
    if (first === undefined) return out
    const origin = Date.parse(first.sim_time)
    const pnl = cumulativePnl(points)
    points.forEach((point, i) => {
      const elapsed = (Date.parse(point.sim_time) - origin) / MS_PER_HOUR
      out.set(elapsed, { pnl: pnl[i] ?? 0, greek: point[metric] })
    })
    return out
  }
  const sideA = side(a)
  const sideB = side(b)
  const hours = [...new Set([...sideA.keys(), ...sideB.keys()])].sort((x, y) => x - y)
  return hours.map((h) => ({
    hours: h,
    pnlA: sideA.get(h)?.pnl ?? null,
    pnlB: sideB.get(h)?.pnl ?? null,
    greekA: sideA.get(h)?.greek ?? null,
    greekB: sideB.get(h)?.greek ?? null,
  }))
}
