import type { ReactElement } from 'react'

import type { LimitKind } from '../api/generated/LimitKind'
import type { LimitStatus } from '../api/generated/LimitStatus'
import type { PortfolioTotals } from '../api/generated/PortfolioTotals'
import {
  formatCompactCurrency,
  formatMicros,
  formatPercent,
  formatPerDay,
  formatPerVolPoint,
  signColor,
} from '../lib/format'
import { limitBarClass, portfolioLimit } from '../lib/limits'

interface LimitedTileDef {
  key: string
  label: string
  limitKind: LimitKind
  tooltip: string
  value: (totals: PortfolioTotals) => number
  format: (value: number) => string
}

const LIMITED_TILES: LimitedTileDef[] = [
  {
    key: 'dollar_delta',
    label: 'Net $ delta',
    limitKind: 'portfolio_dollar_delta',
    tooltip: 'Net dollar delta: the P&L from a 1% move up in every underlying together.',
    value: (t) => t.dollar_delta,
    format: formatCompactCurrency,
  },
  {
    key: 'dollar_gamma_1pct',
    label: '$ Gamma 1%',
    limitKind: 'portfolio_dollar_gamma1_pct',
    tooltip: 'P&L from gamma if the market moves 1%.',
    value: (t) => t.dollar_gamma_1pct,
    format: formatCompactCurrency,
  },
  {
    key: 'vega',
    label: 'Vega',
    limitKind: 'portfolio_vega',
    tooltip: 'P&L from a 1 vol point rise in implied volatility.',
    value: (t) => t.vega,
    format: formatPerVolPoint,
  },
  {
    key: 'theta',
    label: 'Theta',
    limitKind: 'portfolio_theta_floor',
    tooltip: 'P&L from one calendar day passing, all else equal.',
    value: (t) => t.theta,
    format: formatPerDay,
  },
  {
    key: 'gross_exposure',
    label: 'Gross exposure',
    limitKind: 'gross_exposure',
    tooltip: 'Sum of |dollar delta| across every underlying; offsetting positions do not hide each other.',
    value: (t) => t.gross_exposure,
    format: formatCompactCurrency,
  },
]

function LimitedTile({ def, totals, limits }: { def: LimitedTileDef; totals: PortfolioTotals; limits: LimitStatus[] }): ReactElement {
  const value = def.value(totals)
  const limit = portfolioLimit(limits, def.limitKind)
  return (
    <div
      className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
      title={def.tooltip}
      data-testid={`tile-${def.key}`}
      data-state={limit?.state ?? 'ok'}
    >
      <span className="text-xs uppercase tracking-wide text-neutral">{def.label}</span>
      <span className={`text-lg font-semibold tabular-nums ${signColor(value)}`}>{def.format(value)}</span>
      {limit && (
        <>
          <div className="h-1.5 w-full overflow-hidden rounded bg-surface">
            <div
              className={`h-full ${limitBarClass(limit.state)}`}
              style={{ width: `${Math.min(limit.utilisation, 1) * 100}%` }}
            />
          </div>
          <span className="text-xs text-neutral">
            {formatPercent(limit.utilisation)} of {def.format(limit.limit)}
          </span>
        </>
      )}
    </div>
  )
}

/** The risk tiles panel: net dollar delta, dollar gamma 1%, vega, theta, gross
 * exposure, today's P&L and `compute_micros`, each limited tile with a
 * utilisation bar. Reads only its props, never the store directly (see
 * `PositionsTable`'s doc comment for why). `totals`/`limits`/`pnlToday`/
 * `computeMicros` are `null` before the first snapshot or poll response. */
export function RiskTiles({
  totals,
  limits,
  pnlToday,
  computeMicros,
  className = '',
}: {
  totals: PortfolioTotals | null
  limits: LimitStatus[]
  pnlToday: number | null
  computeMicros: number | null
  className?: string
}): ReactElement {
  return (
    <section
      className={`flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="Risk tiles"
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Risk tiles</h2>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {totals === null ? (
          <p className="col-span-full text-sm text-neutral">Waiting for data…</p>
        ) : (
          <>
            {LIMITED_TILES.map((def) => (
              <LimitedTile key={def.key} def={def} totals={totals} limits={limits} />
            ))}
            <div
              className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
              title="Today's session P&L from full revaluation."
              data-testid="tile-pnl-today"
            >
              <span className="text-xs uppercase tracking-wide text-neutral">Today&apos;s P&amp;L</span>
              <span className={`text-lg font-semibold tabular-nums ${signColor(pnlToday ?? 0)}`}>
                {pnlToday === null ? '—' : formatCompactCurrency(pnlToday)}
              </span>
            </div>
            <div
              className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
              title="Microseconds spent computing the last tick."
              data-testid="tile-compute-micros"
            >
              <span className="text-xs uppercase tracking-wide text-neutral">compute_micros</span>
              <span className="text-lg font-semibold tabular-nums">
                {computeMicros === null ? '—' : formatMicros(computeMicros)}
              </span>
            </div>
          </>
        )}
      </div>
    </section>
  )
}
