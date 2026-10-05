import { useMemo, type ReactElement } from 'react'
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  type TooltipContentProps,
} from 'recharts'

import type { BacktestResults } from '../../api/generated/BacktestResults'
import type { DailyRow } from '../../api/generated/DailyRow'
import {
  BUCKET_SERIES,
  drawdownSeries,
  negKey,
  posKey,
  stackedDomain,
  stackedSeries,
} from '../../lib/backtestSeries'
import { dailyRowsToCsv, downloadText } from '../../lib/backtestCsv'
import {
  CHART_AXIS,
  CHART_GRID,
  CHART_TOOLTIP_BG,
  CHART_TOOLTIP_BORDER,
} from '../../lib/chartTheme'
import { formatCompactCurrency, signColor } from '../../lib/format'
import { describeVarBacktest } from '../../lib/varBacktest'

const TOOLTIP_STYLE = {
  backgroundColor: CHART_TOOLTIP_BG,
  border: `1px solid ${CHART_TOOLTIP_BORDER}`,
  fontSize: 12,
}
const DRAWDOWN_COLOR = '#f87171'
/** The cumulative P&L line: near white, so it stands apart from every bucket colour. */
const CUMULATIVE_LINE = '#f9fafb'

/** Tooltip for the cumulative chart: skips the zero half of each split bucket. */
function CumulativeTooltip({ active, payload, label }: TooltipContentProps): ReactElement | null {
  if (!active) return null
  const entries = payload.filter((entry) => Number(entry.value) !== 0)
  return (
    <div className="rounded border text-xs" style={{ ...TOOLTIP_STYLE, padding: 8 }}>
      <div className="font-medium">{label}</div>
      {entries.map((entry) => (
        <div key={String(entry.dataKey)} className="tabular-nums" style={{ color: entry.color }}>
          {entry.name}: {formatCompactCurrency(Number(entry.value))}
        </div>
      ))}
    </div>
  )
}

function Card({ label, children }: { label: string; children: React.ReactNode }): ReactElement {
  return (
    <div className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3">
      <span className="text-xs uppercase tracking-wide text-neutral">{label}</span>
      {children}
    </div>
  )
}

function Money({ value }: { value: number }): ReactElement {
  return (
    <span className={`whitespace-nowrap text-lg font-semibold tabular-nums ${signColor(value)}`}>
      {formatCompactCurrency(value)}
    </span>
  )
}

/** The summary cards of one run. Shared with the saved-run comparison. */
export function SummaryCards({
  results,
  showBuckets = true,
}: {
  results: BacktestResults
  /** The comparison view turns this off: it has a full P&L by bucket table of its own. */
  showBuckets?: boolean
}): ReactElement {
  const { summary } = results
  const drawdown = summary.max_drawdown
  return (
    <div
      className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4"
      data-testid="summary-cards"
    >
      <Card label="Total P&L">
        <Money value={summary.total.actual} />
        <span className="text-xs text-neutral">{summary.days} days replayed</span>
      </Card>
      <Card label="Max drawdown">
        <Money value={-drawdown.amount} />
        <span className="text-xs text-neutral">
          {drawdown.trough_date
            ? `${drawdown.peak_date ?? 'start'} to ${drawdown.trough_date}`
            : 'no drawdown'}
        </span>
      </Card>
      <Card label="Best day">
        {summary.best_day ? (
          <>
            <Money value={summary.best_day.pnl} />
            <span className="text-xs text-neutral">{summary.best_day.date}</span>
          </>
        ) : (
          <span className="text-neutral">—</span>
        )}
      </Card>
      <Card label="Worst day">
        {summary.worst_day ? (
          <>
            <Money value={summary.worst_day.pnl} />
            <span className="text-xs text-neutral">{summary.worst_day.date}</span>
          </>
        ) : (
          <span className="text-neutral">—</span>
        )}
      </Card>
      <Card label="VaR back-test">
        <p className="text-sm" data-testid="var-backtest-sentence">
          {describeVarBacktest(summary.var_backtest)}
        </p>
      </Card>
      <Card label="Limit breaches">
        <span className="text-lg font-semibold tabular-nums">{summary.days_with_breaches}</span>
        <span className="text-xs text-neutral">days with a limit in breach</span>
      </Card>
      {showBuckets && (
      <Card label="P&L by bucket">
        <ul className="grid grid-cols-2 gap-x-3 text-xs tabular-nums">
          {BUCKET_SERIES.map(({ key, label }) => (
            <li key={key} className="flex justify-between gap-2">
              <span className="text-neutral">{label}</span>
              <span className={`whitespace-nowrap ${signColor(summary.total[key])}`}>
                {formatCompactCurrency(summary.total[key])}
              </span>
            </li>
          ))}
        </ul>
      </Card>
      )}
    </div>
  )
}

function CumulativeChart({ rows }: { rows: DailyRow[] }): ReactElement {
  const data = useMemo(() => stackedSeries(rows), [rows])
  const domain = useMemo(() => stackedDomain(data), [data])
  return (
    <section
      aria-label="Cumulative P&L"
      className="rounded-lg border border-border bg-surface-raised p-4"
    >
      <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-neutral">
        Cumulative P&amp;L
      </h3>
      <ResponsiveContainer width="100%" height={280}>
        <ComposedChart data={data} margin={{ top: 4, right: 8, bottom: 4, left: 4 }}>
          <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" />
          <XAxis dataKey="date" stroke={CHART_AXIS} tick={{ fontSize: 10 }} minTickGap={32} />
          <YAxis
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            width={60}
            domain={domain}
            allowDataOverflow
            tickFormatter={(value) => formatCompactCurrency(Number(value))}
          />
          <Tooltip content={CumulativeTooltip} />
          {BUCKET_SERIES.map(({ key, label, color }) => (
            <Area
              key={posKey(key)}
              type="monotone"
              dataKey={posKey(key)}
              name={label}
              stackId="positive"
              stroke={color}
              fill={color}
              fillOpacity={0.35}
              isAnimationActive={false}
              legendType="none"
            />
          ))}
          {BUCKET_SERIES.map(({ key, label, color }) => (
            <Area
              key={negKey(key)}
              type="monotone"
              dataKey={negKey(key)}
              name={label}
              stackId="negative"
              stroke={color}
              fill={color}
              fillOpacity={0.35}
              isAnimationActive={false}
              legendType="none"
            />
          ))}
          <Line
            type="monotone"
            dataKey="total"
            name="Cumulative P&L"
            stroke={CUMULATIVE_LINE}
            strokeWidth={3.5}
            dot={false}
            isAnimationActive={false}
          />
        </ComposedChart>
      </ResponsiveContainer>
    </section>
  )
}

function DrawdownChart({ rows }: { rows: DailyRow[] }): ReactElement {
  const data = useMemo(() => drawdownSeries(rows), [rows])
  return (
    <section
      aria-label="Drawdown"
      className="rounded-lg border border-border bg-surface-raised p-4"
    >
      <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-neutral">Drawdown</h3>
      <ResponsiveContainer width="100%" height={180}>
        <ComposedChart data={data} margin={{ top: 4, right: 8, bottom: 4, left: 4 }}>
          <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" />
          <XAxis dataKey="date" stroke={CHART_AXIS} tick={{ fontSize: 10 }} minTickGap={32} />
          <YAxis
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            width={60}
            tickFormatter={(value) => formatCompactCurrency(Number(value))}
          />
          <Tooltip
            contentStyle={TOOLTIP_STYLE}
            formatter={(value) => formatCompactCurrency(Number(value))}
          />
          <Area
            type="monotone"
            dataKey="drawdown"
            name="Drawdown"
            stroke={DRAWDOWN_COLOR}
            fill={DRAWDOWN_COLOR}
            fillOpacity={0.3}
            isAnimationActive={false}
          />
        </ComposedChart>
      </ResponsiveContainer>
    </section>
  )
}

function DailyTable({ rows }: { rows: DailyRow[] }): ReactElement {
  return (
    <section
      aria-label="Daily results"
      className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4"
    >
      <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">Daily results</h3>
      <div className="max-h-96 overflow-auto">
        <table className="w-full min-w-max border-collapse text-sm tabular-nums">
          <thead className="sticky top-0 bg-surface-raised">
            <tr>
              <th
                scope="col"
                className="border-b border-border px-2 py-1 text-left font-medium text-neutral"
              >
                Date
              </th>
              {BUCKET_SERIES.map(({ key, label }) => (
                <th
                  key={key}
                  scope="col"
                  className="border-b border-border px-2 py-1 text-right font-medium text-neutral"
                >
                  {label}
                </th>
              ))}
              {['Actual', 'VaR 99%', 'Breaches'].map((label) => (
                <th
                  key={label}
                  scope="col"
                  className="border-b border-border px-2 py-1 text-right font-medium text-neutral"
                >
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr
                key={row.date}
                data-exception={row.exception}
                className={row.exception ? 'bg-breach/20' : undefined}
                title={
                  row.exception
                    ? 'VaR exception: the loss was larger than the start-of-day VaR'
                    : undefined
                }
              >
                <td className="whitespace-nowrap px-2 py-1 text-left">
                  {row.date}
                  {row.exception && (
                    <span className="ml-1 text-xs font-medium text-breach">VaR exception</span>
                  )}
                </td>
                {BUCKET_SERIES.map(({ key }) => (
                  <td
                    key={key}
                    className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row[key])}`}
                  >
                    {formatCompactCurrency(row[key])}
                  </td>
                ))}
                <td
                  className={`whitespace-nowrap px-2 py-1 text-right font-medium ${signColor(row.actual)}`}
                >
                  {formatCompactCurrency(row.actual)}
                </td>
                <td className="whitespace-nowrap px-2 py-1 text-right">
                  {formatCompactCurrency(row.var_99)}
                </td>
                <td className="whitespace-nowrap px-2 py-1 text-right">{row.breaches}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  )
}

/** The results of one run, built only from its `BacktestResults` (never from the stream), so a
 * finished run reads the same after a server restart. A partial run says so. */
export function ResultsView({ results }: { results: BacktestResults }): ReactElement {
  const { rows } = results
  return (
    <div className="flex flex-col gap-4" data-testid="results-view">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold">Results for {results.settings.run_id}</h2>
        <button
          type="button"
          disabled={rows.length === 0}
          onClick={() => downloadText(`${results.settings.run_id}-daily.csv`, dailyRowsToCsv(rows))}
          className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
        >
          Download daily table (CSV)
        </button>
      </div>
      {results.partial && (
        <p
          role="status"
          className="rounded border border-warning bg-warning/10 p-2 text-sm text-warning"
        >
          {results.status.state === 'unfinished'
            ? `This run is unfinished: it stopped after ${rows.length} of ${results.status.steps_total} dates, so these figures cover only those dates.`
            : `This run is still going: these figures cover the first ${rows.length} of ${results.status.steps_total} dates.`}
        </p>
      )}
      <SummaryCards results={results} />
      {rows.length > 0 && (
        <>
          <CumulativeChart rows={rows} />
          <DrawdownChart rows={rows} />
        </>
      )}
      <DailyTable rows={rows} />
    </div>
  )
}
