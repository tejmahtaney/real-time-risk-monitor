import { useEffect, useMemo, useState, type ReactElement } from 'react'

import type { HistoryPoint } from '../../api/generated/HistoryPoint'
import { api } from '../../api/rest'
import { dailyPnlToCsv, historyToCsv } from '../../lib/storageCsv'
import { downloadText } from '../../lib/backtestCsv'
import { BUCKET_SERIES } from '../../lib/backtestSeries'
import { formatCompactCurrency, formatSimTime, signColor } from '../../lib/format'
import {
  describeEvent,
  EVENT_TYPES,
  eventTime,
  rangeBounds,
  sessionDates,
} from '../../lib/sessions'
import { GreeksCharts } from '../GreeksCharts'
import { useLoad } from './useLoad'

/** The most points the server returns for one chart range. */
const CHART_POINTS = 1200

function Section({ title, children }: { title: string; children: React.ReactNode }): ReactElement {
  return (
    <section
      aria-label={title}
      className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4"
    >
      <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">{title}</h3>
      {children}
    </section>
  )
}

function EventsList({ id }: { id: string }): ReactElement {
  const [type, setType] = useState('')
  const events = useLoad(() => api.sessionEvents(id, type === '' ? undefined : type), [id, type])
  return (
    <Section title="Events">
      <label className="flex items-center gap-2 text-sm">
        Type
        <select
          value={type}
          onChange={(event) => setType(event.target.value)}
          className="rounded border border-border bg-surface px-2 py-1"
        >
          <option value="">All</option>
          {EVENT_TYPES.map((t) => (
            <option key={t} value={t}>
              {t.replace('_', ' ')}
            </option>
          ))}
        </select>
      </label>
      {events.error && <p role="alert" className="text-sm text-down">{events.error}</p>}
      {events.data?.length === 0 && <p className="text-sm text-neutral">No events of this type.</p>}
      <ul className="max-h-80 overflow-auto text-sm" data-testid="events-list">
        {events.data?.map((event, i) => (
          <li key={i} className="flex gap-3 border-b border-border py-1">
            <span className="w-36 shrink-0 tabular-nums text-neutral">
              {formatSimTime(eventTime(event), { withDate: true })}
            </span>
            <span className="w-32 shrink-0 text-neutral">{event.type.replace('_', ' ')}</span>
            <span>{describeEvent(event)}</span>
          </li>
        ))}
      </ul>
    </Section>
  )
}

/** One saved session: Greeks over a chosen range of simulated dates, every daily P&L bucket,
 * the events, and CSV exports of the daily table and the selected range. */
export function SessionDetail({ id, onBack }: { id: string; onBack: () => void }): ReactElement {
  const detail = useLoad(() => api.sessionDetail(id), [id])
  const overview = useLoad(() => api.sessionHistory(id, { maxPoints: CHART_POINTS }), [id])
  const dates = useMemo(() => sessionDates(overview.data ?? []), [overview.data])
  const [fromDate, setFromDate] = useState('')
  const [toDate, setToDate] = useState('')
  const [range, setRange] = useState<HistoryPoint[] | null>(null)
  const [exportError, setExportError] = useState<string | null>(null)

  const first = dates[0]?.date ?? ''
  const last = dates[dates.length - 1]?.date ?? ''
  const from = fromDate === '' ? first : fromDate
  const to = toDate === '' ? last : toDate
  const bounds = useMemo(() => rangeBounds(dates, from, to), [dates, from, to])

  // Ask the server for just the chosen range, downsampled.
  useEffect(() => {
    if (bounds === null) return
    let cancelled = false
    api
      .sessionHistory(id, { maxPoints: CHART_POINTS, ...bounds })
      .then((points) => {
        if (!cancelled) setRange(points)
      })
      .catch(() => {
        if (!cancelled) setRange([])
      })
    return () => {
      cancelled = true
    }
  }, [id, bounds])

  const exportRange = async (): Promise<void> => {
    if (bounds === null) return
    setExportError(null)
    try {
      // `maxPoints: 0` asks for every point, not the downsampled chart data.
      const points = await api.sessionHistory(id, { maxPoints: 0, ...bounds })
      downloadText(`${id}-history-${from}-to-${to}.csv`, historyToCsv(points))
    } catch (err) {
      setExportError(err instanceof Error ? err.message : String(err))
    }
  }

  const rows = detail.data?.daily_pnl ?? []
  const meta = detail.data?.meta

  return (
    <div className="flex flex-col gap-4" data-testid="session-detail">
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" onClick={onBack} className="rounded border border-border px-3 py-1 text-sm hover:bg-surface">
          Back to sessions
        </button>
        <h2 className="text-base font-semibold">
          Session <span className="font-mono text-sm">{id}</span>
        </h2>
        {detail.data?.running === true && (
          <span className="rounded bg-up/20 px-1.5 py-0.5 text-xs text-up">running</span>
        )}
        {meta && meta.end === null && detail.data?.running !== true && (
          <span className="rounded bg-warning/20 px-1.5 py-0.5 text-xs text-warning">cut short</span>
        )}
      </div>
      {detail.error && <p role="alert" className="text-sm text-down">{detail.error}</p>}

      <Section title="Range">
        {dates.length === 0 ? (
          <p className="text-sm text-neutral">{overview.data ? 'This session recorded no history.' : 'Loading…'}</p>
        ) : (
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <label className="flex items-center gap-2">
              From
              <select
                value={from}
                onChange={(event) => setFromDate(event.target.value)}
                className="rounded border border-border bg-surface px-2 py-1"
              >
                {dates.map((d) => (
                  <option key={d.date} value={d.date}>
                    {d.date}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex items-center gap-2">
              To
              <select
                value={to}
                onChange={(event) => setToDate(event.target.value)}
                className="rounded border border-border bg-surface px-2 py-1"
              >
                {dates.map((d) => (
                  <option key={d.date} value={d.date}>
                    {d.date}
                  </option>
                ))}
              </select>
            </label>
            {bounds === null && <span className="text-down">The start date is after the end date.</span>}
            <button
              type="button"
              disabled={bounds === null}
              onClick={() => void exportRange()}
              className="ml-auto rounded border border-border px-3 py-1 hover:bg-surface disabled:opacity-50"
            >
              Download range (CSV)
            </button>
          </div>
        )}
        {exportError && <p role="alert" className="text-sm text-down">{exportError}</p>}
      </Section>

      {dates.length > 0 && <GreeksCharts history={range ?? []} stored />}

      <Section title="Daily P&L">
        <div className="flex justify-end">
          <button
            type="button"
            disabled={rows.length === 0}
            onClick={() => downloadText(`${id}-daily-pnl.csv`, dailyPnlToCsv(rows))}
            className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
          >
            Download daily P&amp;L (CSV)
          </button>
        </div>
        {rows.length === 0 ? (
          <p className="text-sm text-neutral">No day has closed in this session yet.</p>
        ) : (
          <div className="max-h-96 overflow-auto">
            <table className="w-full min-w-max border-collapse text-sm tabular-nums">
              <thead className="sticky top-0 bg-surface-raised">
                <tr>
                  <th scope="col" className="border-b border-border px-2 py-1 text-left font-medium text-neutral">Date</th>
                  {BUCKET_SERIES.map(({ key, label }) => (
                    <th key={key} scope="col" className="border-b border-border px-2 py-1 text-right font-medium text-neutral">
                      {label}
                    </th>
                  ))}
                  <th scope="col" className="border-b border-border px-2 py-1 text-right font-medium text-neutral">Actual</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr key={row.session_date}>
                    <td className="whitespace-nowrap px-2 py-1">{row.session_date}</td>
                    {BUCKET_SERIES.map(({ key }) => (
                      <td key={key} className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row[key])}`}>
                        {formatCompactCurrency(row[key])}
                      </td>
                    ))}
                    <td className={`whitespace-nowrap px-2 py-1 text-right font-medium ${signColor(row.actual)}`}>
                      {formatCompactCurrency(row.actual)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <EventsList id={id} />
    </div>
  )
}
