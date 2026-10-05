import { useMemo, type ReactElement } from 'react'
import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import { api } from '../../api/rest'
import { overlaySeries } from '../../lib/backtestCompare'
import { BUCKET_SERIES } from '../../lib/backtestSeries'
import {
  CHART_AXIS,
  CHART_GRID,
  CHART_LINE,
  CHART_TOOLTIP_BG,
  CHART_TOOLTIP_BORDER,
} from '../../lib/chartTheme'
import { formatCompactCurrency, signColor } from '../../lib/format'
import { useLoad } from '../storage/useLoad'
import { SummaryCards } from './ResultsView'

const COLOR_B = '#fbbf24'

/** Two finished saved runs side by side: their summary cards, an overlay of cumulative P&L and
 * the P&L by bucket for each. Reads each run's `GET /results` only: no lease, no stream. */
export function CompareRuns({
  a,
  b,
  onBack,
}: {
  a: string
  b: string
  onBack: () => void
}): ReactElement {
  const results = useLoad(
    () => Promise.all([api.backtestResults(a), api.backtestResults(b)]),
    [a, b],
  )
  const runs = results.data
  const overlay = useMemo(
    () => (results.data ? overlaySeries(results.data[0].rows, results.data[1].rows) : []),
    [results.data],
  )

  return (
    <div className="flex flex-col gap-4" data-testid="compare-runs">
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" onClick={onBack} className="rounded border border-border px-3 py-1 text-sm hover:bg-surface">
          Back to saved runs
        </button>
        <h2 className="text-base font-semibold">
          Comparing <span className="font-mono text-sm">{a}</span> and <span className="font-mono text-sm">{b}</span>
        </h2>
      </div>
      {results.error && <p role="alert" className="text-sm text-down">{results.error}</p>}
      {!results.data && !results.error && <p className="text-sm text-neutral">Loading the results…</p>}
      {runs && (
        <>
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            {runs.map((run, i) => (
              <section key={run.settings.run_id} aria-label={`Run ${i === 0 ? 'A' : 'B'} summary`} className="flex flex-col gap-2">
                <h3 className="text-sm font-semibold" style={{ color: i === 0 ? CHART_LINE : COLOR_B }}>
                  {i === 0 ? 'A' : 'B'}: <span className="font-mono">{run.settings.run_id}</span>
                </h3>
                <SummaryCards results={run} />
              </section>
            ))}
          </div>

          <section aria-label="Cumulative P&L overlay" className="rounded-lg border border-border bg-surface-raised p-4">
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-neutral">Cumulative P&amp;L</h3>
            <ResponsiveContainer width="100%" height={280}>
              <LineChart data={overlay} margin={{ top: 4, right: 8, bottom: 4, left: 4 }}>
                <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" />
                <XAxis dataKey="date" stroke={CHART_AXIS} tick={{ fontSize: 10 }} minTickGap={32} />
                <YAxis
                  domain={['auto', 'auto']}
                  stroke={CHART_AXIS}
                  tick={{ fontSize: 10 }}
                  width={60}
                  tickFormatter={(v) => formatCompactCurrency(Number(v))}
                />
                <Tooltip
                  formatter={(v) => formatCompactCurrency(Number(v))}
                  contentStyle={{ backgroundColor: CHART_TOOLTIP_BG, border: `1px solid ${CHART_TOOLTIP_BORDER}`, fontSize: 12 }}
                />
                <Legend wrapperStyle={{ fontSize: 11 }} />
                <Line type="monotone" dataKey="a" name={a} stroke={CHART_LINE} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />
                <Line type="monotone" dataKey="b" name={b} stroke={COLOR_B} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />
              </LineChart>
            </ResponsiveContainer>
          </section>

          <section aria-label="P&L by bucket" className="rounded-lg border border-border bg-surface-raised p-4">
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-neutral">P&amp;L by bucket</h3>
            <table className="w-full max-w-xl border-collapse text-sm tabular-nums">
              <thead>
                <tr className="text-left text-neutral">
                  <th className="px-2 py-1 font-medium">Bucket</th>
                  <th className="px-2 py-1 text-right font-medium">A</th>
                  <th className="px-2 py-1 text-right font-medium">B</th>
                </tr>
              </thead>
              <tbody>
                {BUCKET_SERIES.map(({ key, label }) => {
                  const [ra, rb] = runs
                  return (
                    <tr key={key} data-testid={`bucket-${key}`}>
                      <td className="px-2 py-1">{label}</td>
                      <td className={`px-2 py-1 text-right ${signColor(ra.summary.total[key])}`}>
                        {formatCompactCurrency(ra.summary.total[key])}
                      </td>
                      <td className={`px-2 py-1 text-right ${signColor(rb.summary.total[key])}`}>
                        {formatCompactCurrency(rb.summary.total[key])}
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </section>
        </>
      )}
    </div>
  )
}
