import { useMemo, useState, type ReactElement } from 'react'
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  type TooltipContentProps,
} from 'recharts'

import type { PnlBuckets } from '../api/generated/PnlBuckets'
import type { PnlReport } from '../api/generated/PnlReport'
import { CHART_AXIS, CHART_GRID, CHART_TOOLTIP_BG, CHART_TOOLTIP_BORDER } from '../lib/chartTheme'
import { formatCompactCurrency } from '../lib/format'
import { buildHeadline } from '../lib/headline'
import {
  PNL_PERIODS,
  STACK_KEYS,
  bucketChartHeight,
  bucketRows,
  portfolioBucketsForPeriod,
  underlyingStacks,
  type BucketRow,
  type PnlPeriod,
} from '../lib/pnlAttribution'
import { symbolColor } from '../lib/symbolColors'

const PERIOD_LABEL: Record<PnlPeriod, string> = {
  today: 'Today',
  since_start: 'Since start',
  previous_day: 'Previous day',
}

const GAIN_COLOR = '#34d399' // --color-up
const LOSS_COLOR = '#f87171' // --color-down
const NEUTRAL_COLOR = '#9ca3af' // --color-neutral

const DIVIDENDS_EXPLANATION =
  "Cash received on an ex-date. The stock's price drop shows in Delta, and this is the cash that offsets it, so a holder's total is not distorted."

const RESIDUAL_EXPLANATION =
  "The third-order and cross terms the Greeks above don't capture. Grows with step size and near expiry — not an error to hide."

/** A bucket's bar colour: green for a gain, red for a loss, neutral for zero — and
 * always neutral for `residual`, whichever sign it happens to carry, since it is
 * leftover, not a Greek exposure. */
function bucketColor(key: keyof PnlBuckets, value: number): string {
  if (key === 'residual') return NEUTRAL_COLOR
  if (value > 0) return GAIN_COLOR
  if (value < 0) return LOSS_COLOR
  return NEUTRAL_COLOR
}

function AttributionTooltip({ active, payload }: TooltipContentProps): ReactElement | null {
  const entry = active ? payload[0] : undefined
  if (!entry) return null
  const row = entry.payload as BucketRow
  return (
    <div
      className="max-w-56 rounded border text-xs"
      style={{ backgroundColor: CHART_TOOLTIP_BG, borderColor: CHART_TOOLTIP_BORDER, padding: 8 }}
    >
      <div className="font-medium">{row.label}</div>
      <div className="tabular-nums">{formatCompactCurrency(row.value)}</div>
      {row.key === 'residual' && <p className="mt-1 text-neutral">{RESIDUAL_EXPLANATION}</p>}
      {row.key === 'dividends' && <p className="mt-1 text-neutral">{DIVIDENDS_EXPLANATION}</p>}
    </div>
  )
}

/** The P&L attribution panel: the plain-English headline (from today's buckets),
 * a horizontal bar per bucket for the chosen period, and a small per-underlying
 * stacked breakdown. Reads only its `pnl` prop, never the store directly. */
export function PnlAttributionPanel({
  pnl,
  className = '',
}: {
  pnl: PnlReport | null
  className?: string
}): ReactElement {
  const [period, setPeriod] = useState<PnlPeriod>('today')

  const buckets = pnl ? portfolioBucketsForPeriod(pnl, period) : null
  const rows = useMemo(() => (buckets ? bucketRows(buckets) : []), [buckets])
  const stacks = useMemo(() => (pnl ? underlyingStacks(pnl, period) : []), [pnl, period])
  const headline = pnl ? buildHeadline(pnl.portfolio.today) : null

  return (
    <section
      className={`flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="P&L attribution"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">
          P&amp;L attribution
        </h2>
        <div className="flex gap-1" role="group" aria-label="Period">
          {PNL_PERIODS.map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => setPeriod(p)}
              aria-pressed={period === p}
              className={`rounded px-2 py-1 text-xs ${
                period === p
                  ? 'bg-surface font-semibold text-neutral'
                  : 'text-neutral hover:bg-surface'
              }`}
            >
              {PERIOD_LABEL[p]}
            </button>
          ))}
        </div>
      </div>

      <p className="text-sm" data-testid="pnl-headline">
        {headline ?? 'Waiting for data…'}
      </p>

      {rows.length === 0 ? (
        <p className="text-sm text-neutral">
          No P&amp;L data for {PERIOD_LABEL[period].toLowerCase()} yet.
        </p>
      ) : (
        <>
          <ResponsiveContainer width="100%" height={bucketChartHeight(rows.length)}>
            <BarChart
              data={rows}
              layout="vertical"
              margin={{ top: 4, right: 16, bottom: 4, left: 4 }}
            >
              <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" horizontal={false} />
              <XAxis
                type="number"
                stroke={CHART_AXIS}
                tick={{ fontSize: 10 }}
                tickFormatter={formatCompactCurrency}
              />
              <YAxis
                type="category"
                dataKey="label"
                stroke={CHART_AXIS}
                tick={{ fontSize: 11 }}
                width={78}
                interval={0}
              />
              <Tooltip content={AttributionTooltip} cursor={{ fill: 'transparent' }} />
              <Bar dataKey="value" isAnimationActive={false}>
                {rows.map((row) => (
                  <Cell key={row.key} fill={bucketColor(row.key, row.value)} />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>

          {stacks.length > 0 && (
            <div className="flex flex-col gap-1">
              <h3 className="text-xs font-semibold uppercase tracking-wide text-neutral">
                By underlying
              </h3>
              <ResponsiveContainer width="100%" height={Math.max(100, stacks.length * 32)}>
                <BarChart
                  data={stacks.map((s) => ({ symbol: s.symbol, ...s.values }))}
                  layout="vertical"
                  margin={{ top: 4, right: 16, bottom: 4, left: 4 }}
                >
                  <CartesianGrid stroke={CHART_GRID} strokeDasharray="3 3" horizontal={false} />
                  <XAxis
                    type="number"
                    stroke={CHART_AXIS}
                    tick={{ fontSize: 10 }}
                    tickFormatter={formatCompactCurrency}
                  />
                  <YAxis
                    type="category"
                    dataKey="symbol"
                    stroke={CHART_AXIS}
                    tick={{ fontSize: 11 }}
                    width={50}
                    interval={0}
                  />
                  <Tooltip
                    formatter={(value, name) => [
                      formatCompactCurrency(Number(value)),
                      String(name),
                    ]}
                    contentStyle={{
                      backgroundColor: CHART_TOOLTIP_BG,
                      border: `1px solid ${CHART_TOOLTIP_BORDER}`,
                      fontSize: 12,
                    }}
                  />
                  {STACK_KEYS.map((key) => (
                    <Bar key={key} dataKey={key} stackId="pnl" isAnimationActive={false}>
                      {stacks.map((row) => (
                        <Cell
                          key={`${key}-${row.symbol}`}
                          fill={bucketColor(key, row.values[key] ?? 0)}
                          stroke={symbolColor(row.symbol)}
                          strokeWidth={1}
                        />
                      ))}
                    </Bar>
                  ))}
                </BarChart>
              </ResponsiveContainer>
            </div>
          )}
        </>
      )}
    </section>
  )
}
