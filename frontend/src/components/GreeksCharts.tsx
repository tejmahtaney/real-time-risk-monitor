import { useMemo, useState, type ReactElement } from 'react'
import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceArea,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { LimitStatus } from '../api/generated/LimitStatus'
import { computeChartScale, metricBounds, type MetricBounds } from '../lib/chartLimits'
import { CHART_AXIS, CHART_GRID, CHART_LIMIT_LINE, CHART_LINE, CHART_TOOLTIP_BG, CHART_TOOLTIP_BORDER, CHART_WARNING_BAND } from '../lib/chartTheme'
import { CHART_WINDOWS, capHistory, filterWindow, type ChartWindow } from '../lib/chartWindow'
import { formatCompactCurrency, formatSimTime } from '../lib/format'
import type { GreekMetric } from '../lib/chartLimits'

interface SeriesPoint {
  t: number
  value: number
}

// All four measures are dollar amounts (vega and theta are per vol point / per day
// respectively, but still $-denominated); the unit itself lives in the title, not
// the axis or tooltip, so both stay short (`$1.2k`) — a long "$1.2k / vol pt" tick
// on the vega chart was what made its labels overlap the shared time axis below.
const METRICS: Array<{ key: GreekMetric; title: string }> = [
  { key: 'dollar_delta', title: 'Dollar delta ($)' },
  { key: 'dollar_gamma_1pct', title: 'Dollar gamma 1% ($)' },
  { key: 'vega', title: 'Vega ($ / vol pt)' },
  { key: 'theta', title: 'Theta ($ / day)' },
  { key: 'var_99', title: 'VaR 99% ($)' },
]

function axisTickLabel(t: number): string {
  return formatSimTime(new Date(t).toISOString(), { seconds: false })
}

function tooltipTimeLabel(t: number): string {
  return formatSimTime(new Date(t).toISOString())
}

/** One metric's chart. The y-axis domain, whether a limit is drawn (and on which
 * side), and whether it's shown as an offscreen corner label instead all come from
 * {@link computeChartScale} — see that function's doc comment for why the domain
 * is data-only, not a mirrored ±limit. */
function GreekMiniChart({
  title,
  data,
  bounds,
}: {
  title: string
  data: SeriesPoint[]
  bounds: MetricBounds
}): ReactElement {
  const scale = useMemo(() => computeChartScale(data.map((d) => d.value), bounds), [data, bounds])
  const bound = scale.visibleBound
  const bandLow = bound ? Math.min(bound.limit, bound.warningStart) : null
  const bandHigh = bound ? Math.max(bound.limit, bound.warningStart) : null

  return (
    <div className="flex flex-col gap-1 rounded-lg border border-border bg-surface p-2" data-testid={`greek-chart-${title}`}>
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-medium uppercase tracking-wide text-neutral">{title}</h3>
        {scale.offscreenLimit !== null && (
          <span
            className="whitespace-nowrap text-[10px] text-neutral"
            data-testid="offscreen-limit"
            title="The configured limit is far outside this chart's current range, so it isn't drawn on the axis."
          >
            limit {formatCompactCurrency(scale.offscreenLimit)}
          </span>
        )}
      </div>
      <ResponsiveContainer width="100%" height={140}>
        <LineChart data={data} syncId="greeks-over-time" margin={{ top: 4, right: 8, bottom: 4, left: 4 }}>
          <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" />
          <XAxis
            dataKey="t"
            type="number"
            domain={['dataMin', 'dataMax']}
            tickFormatter={axisTickLabel}
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            minTickGap={24}
          />
          <YAxis
            domain={scale.domain}
            stroke={CHART_AXIS}
            tick={{ fontSize: 10 }}
            width={56}
            tickFormatter={(value) => formatCompactCurrency(Number(value))}
          />
          <Tooltip
            labelFormatter={(label) => tooltipTimeLabel(Number(label))}
            formatter={(value) => formatCompactCurrency(Number(value))}
            contentStyle={{ backgroundColor: CHART_TOOLTIP_BG, border: `1px solid ${CHART_TOOLTIP_BORDER}`, fontSize: 12 }}
          />
          {bound && bandLow !== null && bandHigh !== null && (
            <>
              <ReferenceArea y1={bandLow} y2={bandHigh} fill={CHART_WARNING_BAND} fillOpacity={0.15} strokeOpacity={0} />
              <ReferenceLine y={bound.limit} stroke={CHART_LIMIT_LINE} strokeDasharray="4 4" />
            </>
          )}
          <Line type="monotone" dataKey="value" stroke={CHART_LINE} strokeWidth={2} dot={false} isAnimationActive={false} />
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}

/** Five small line charts (dollar delta, dollar gamma 1%, vega, theta, headline
 * VaR99 — Part 18), sharing one time axis via `syncId` so hovering one crosshairs
 * all five. `history` is expected to come from `Dashboard`'s `useThrottledHistory`,
 * so a fast tick rate cannot redraw these more than twice a second; each metric's
 * array is then derived with `useMemo` so an unrelated re-render does not
 * recompute five data arrays. Reads only its props, never the store directly (see
 * `PositionsTable`'s doc comment for why). */
export function GreeksCharts({
  history,
  limits,
  className = '',
}: {
  history: HistoryPoint[]
  limits: LimitStatus[]
  className?: string
}): ReactElement {
  const [window, setWindow] = useState<ChartWindow>('15m')

  const windowed = useMemo(() => capHistory(filterWindow(history, window)), [history, window])

  const series = useMemo(() => {
    const byMetric: Record<GreekMetric, SeriesPoint[]> = {
      dollar_delta: [],
      dollar_gamma_1pct: [],
      vega: [],
      theta: [],
      var_99: [],
    }
    for (const point of windowed) {
      const t = Date.parse(point.sim_time)
      if (Number.isNaN(t)) continue
      byMetric.dollar_delta.push({ t, value: point.dollar_delta })
      byMetric.dollar_gamma_1pct.push({ t, value: point.dollar_gamma_1pct })
      byMetric.vega.push({ t, value: point.vega })
      byMetric.theta.push({ t, value: point.theta })
      // `var_99` is `null` only on a server built before Part 18 or a not-yet-live
      // point; skip it rather than plotting a fake zero.
      if (point.var_99 !== null) byMetric.var_99.push({ t, value: point.var_99 })
    }
    return byMetric
  }, [windowed])

  return (
    <section
      className={`flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="Greeks over time"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Greeks over time</h2>
        <div className="flex gap-1" role="group" aria-label="Time window">
          {CHART_WINDOWS.map((w) => (
            <button
              key={w}
              type="button"
              onClick={() => setWindow(w)}
              aria-pressed={window === w}
              className={`rounded px-2 py-1 text-xs ${
                window === w ? 'bg-surface font-semibold text-neutral' : 'text-neutral hover:bg-surface'
              }`}
            >
              {w === 'all' ? 'All' : w}
            </button>
          ))}
        </div>
      </div>
      {windowed.length === 0 ? (
        <p className="text-sm text-neutral">Waiting for history…</p>
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          {METRICS.map((metric) => (
            <GreekMiniChart
              key={metric.key}
              title={metric.title}
              data={series[metric.key]}
              bounds={metricBounds(limits, metric.key)}
            />
          ))}
        </div>
      )}
    </section>
  )
}
