import { useMemo, useState, type ReactElement } from 'react'
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from 'recharts'

import type { Concentration } from '../api/generated/Concentration'
import type { Symbol } from '../api/generated/Symbol'
import type { UnderlyingRisk } from '../api/generated/UnderlyingRisk'
import { allocationSlices, allocationTotal, type AllocationMeasure } from '../lib/allocation'
import { CHART_TOOLTIP_BG, CHART_TOOLTIP_BORDER } from '../lib/chartTheme'
import { formatCompactCurrency, formatPercent } from '../lib/format'
import { symbolColor } from '../lib/symbolColors'

const MEASURES: Array<{ key: AllocationMeasure; label: string }> = [
  { key: 'gross_exposure', label: 'Gross exposure' },
  { key: 'vega', label: '|Vega|' },
]

/** Share of the book by underlying, as a donut: gross exposure (from
 * `RiskSummary.concentration`) or |vega| (derived here). A fixed colour per symbol
 * (`symbolColor`, shared with `PositionsTable`'s group headers) makes a symbol
 * recognisable across both, and the centre label gives the total so the donut
 * still reads as a number, not just relative shares. Reads only its props, never
 * the store directly. */
export function AllocationDonut({
  concentration,
  byUnderlying,
  className = '',
}: {
  concentration: Concentration[]
  byUnderlying: Record<Symbol, UnderlyingRisk>
  className?: string
}): ReactElement {
  const [measure, setMeasure] = useState<AllocationMeasure>('gross_exposure')
  const slices = useMemo(
    () => allocationSlices(measure, concentration, byUnderlying),
    [measure, concentration, byUnderlying],
  )
  const total = allocationTotal(slices)

  return (
    <section
      className={`flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="Allocation"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Allocation</h2>
        <div className="flex gap-1" role="group" aria-label="Allocation measure">
          {MEASURES.map((m) => (
            <button
              key={m.key}
              type="button"
              onClick={() => setMeasure(m.key)}
              aria-pressed={measure === m.key}
              className={`rounded px-2 py-1 text-xs ${
                measure === m.key ? 'bg-surface font-semibold text-neutral' : 'text-neutral hover:bg-surface'
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>

      {slices.length === 0 ? (
        <p className="text-sm text-neutral">No positions.</p>
      ) : (
        <div className="flex flex-col items-center gap-3 sm:flex-row">
          <div className="relative h-48 w-48 shrink-0">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={slices}
                  dataKey="value"
                  nameKey="symbol"
                  innerRadius="62%"
                  outerRadius="90%"
                  paddingAngle={slices.length > 1 ? 2 : 0}
                  isAnimationActive={false}
                >
                  {slices.map((slice) => (
                    <Cell key={slice.symbol} fill={symbolColor(slice.symbol)} />
                  ))}
                </Pie>
                <Tooltip
                  formatter={(value, _name, item) => {
                    const fraction = (item.payload as { fraction?: number } | undefined)?.fraction ?? 0
                    return [`${formatCompactCurrency(Number(value))} (${formatPercent(fraction)})`, 'value']
                  }}
                  labelFormatter={(symbol) => String(symbol)}
                  contentStyle={{ backgroundColor: CHART_TOOLTIP_BG, border: `1px solid ${CHART_TOOLTIP_BORDER}`, fontSize: 12 }}
                />
              </PieChart>
            </ResponsiveContainer>
            <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
              <span className="text-lg font-semibold tabular-nums" data-testid="allocation-total">
                {formatCompactCurrency(total)}
              </span>
              <span className="text-xs text-neutral">Total</span>
            </div>
          </div>

          <ul className="flex w-full flex-col gap-1 text-sm" data-testid="allocation-legend">
            {slices.map((slice) => (
              <li key={slice.symbol} className="flex items-center justify-between gap-2">
                <span className="flex items-center gap-2">
                  <span
                    className="inline-block h-2.5 w-2.5 rounded-full"
                    style={{ backgroundColor: symbolColor(slice.symbol) }}
                    aria-hidden
                  />
                  {slice.symbol}
                </span>
                <span className="tabular-nums text-neutral">
                  {formatCompactCurrency(slice.value)} · {formatPercent(slice.fraction)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  )
}
