import { useMemo, useState, type ReactElement } from 'react'
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
import { CHART_AXIS, CHART_GRID, CHART_LINE, CHART_TOOLTIP_BG, CHART_TOOLTIP_BORDER } from '../../lib/chartTheme'
import { formatCompactCurrency } from '../../lib/format'
import { alignSessions, type ComparePoint, type CompareMetric } from '../../lib/sessions'
import { useLoad } from './useLoad'

const METRICS: Array<{ key: CompareMetric; label: string }> = [
  { key: 'dollar_delta', label: 'Dollar delta' },
  { key: 'dollar_gamma_1pct', label: 'Dollar gamma 1%' },
  { key: 'vega', label: 'Vega' },
  { key: 'theta', label: 'Theta' },
  { key: 'var_99', label: 'VaR 99%' },
]
const COLOR_B = '#fbbf24'

function CompareChart({
  title,
  data,
  keyA,
  keyB,
  idA,
  idB,
}: {
  title: string
  data: ComparePoint[]
  keyA: keyof ComparePoint
  keyB: keyof ComparePoint
  idA: string
  idB: string
}): ReactElement {
  return (
    <section aria-label={title} className="rounded-lg border border-border bg-surface-raised p-4">
      <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-neutral">{title}</h3>
      <ResponsiveContainer width="100%" height={240}>
        <LineChart data={data} margin={{ top: 4, right: 8, bottom: 16, left: 4 }}>
          <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" />
          <XAxis
            dataKey="hours"
            type="number"
            domain={['dataMin', 'dataMax']}
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            tickFormatter={(h) => `${Number(h).toFixed(0)} h`}
            label={{ value: 'Simulated hours since session start', position: 'insideBottom', offset: -8, fontSize: 10, fill: CHART_AXIS }}
          />
          <YAxis
            domain={['auto', 'auto']}
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            width={60}
            tickFormatter={(v) => formatCompactCurrency(Number(v))}
          />
          <Tooltip
            labelFormatter={(h) => `${Number(h).toFixed(2)} h in`}
            formatter={(v) => formatCompactCurrency(Number(v))}
            contentStyle={{ backgroundColor: CHART_TOOLTIP_BG, border: `1px solid ${CHART_TOOLTIP_BORDER}`, fontSize: 12 }}
          />
          <Legend wrapperStyle={{ fontSize: 11 }} />
          <Line type="monotone" dataKey={keyA} name={idA} stroke={CHART_LINE} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />
          <Line type="monotone" dataKey={keyB} name={idB} stroke={COLOR_B} strokeWidth={2} dot={false} connectNulls isAnimationActive={false} />
        </LineChart>
      </ResponsiveContainer>
    </section>
  )
}

/** Two sessions on the same charts: cumulative P&L and one chosen Greek, aligned by simulated
 * time elapsed since each session's own first history point. */
export function SessionCompare({
  a,
  b,
  onBack,
}: {
  a: string
  b: string
  onBack: () => void
}): ReactElement {
  const [metric, setMetric] = useState<CompareMetric>('dollar_delta')
  const histories = useLoad(
    () =>
      Promise.all([
        api.sessionHistory(a, { maxPoints: 2000 }),
        api.sessionHistory(b, { maxPoints: 2000 }),
      ]),
    [a, b],
  )
  const data = useMemo(
    () => (histories.data ? alignSessions(histories.data[0], histories.data[1], metric) : []),
    [histories.data, metric],
  )
  const label = METRICS.find((m) => m.key === metric)?.label ?? metric

  return (
    <div className="flex flex-col gap-4" data-testid="session-compare">
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" onClick={onBack} className="rounded border border-border px-3 py-1 text-sm hover:bg-surface">
          Back to sessions
        </button>
        <h2 className="text-base font-semibold">
          Comparing <span className="font-mono text-sm">{a}</span> and <span className="font-mono text-sm">{b}</span>
        </h2>
        <label className="ml-auto flex items-center gap-2 text-sm">
          Greek
          <select
            value={metric}
            onChange={(event) => setMetric(event.target.value as CompareMetric)}
            className="rounded border border-border bg-surface px-2 py-1"
          >
            {METRICS.map((m) => (
              <option key={m.key} value={m.key}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      {histories.error && <p role="alert" className="text-sm text-down">{histories.error}</p>}
      {histories.data && data.length === 0 && <p className="text-sm text-neutral">Neither session recorded any history.</p>}
      {data.length > 0 && (
        <>
          <CompareChart title="Cumulative P&L" data={data} keyA="pnlA" keyB="pnlB" idA={a} idB={b} />
          <CompareChart title={label} data={data} keyA="greekA" keyB="greekB" idA={a} idB={b} />
        </>
      )}
    </div>
  )
}
