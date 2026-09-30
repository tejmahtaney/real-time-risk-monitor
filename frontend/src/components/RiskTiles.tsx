import type { ReactElement } from 'react'

import type { LimitKind } from '../api/generated/LimitKind'
import type { LimitStatus } from '../api/generated/LimitStatus'
import type { PortfolioTotals } from '../api/generated/PortfolioTotals'
import type { VarMethod } from '../api/generated/VarMethod'
import type { VarMethodReport } from '../api/generated/VarMethodReport'
import type { VarReport } from '../api/generated/VarReport'
import { formatCompactCurrency, formatMicros, formatPercent, signColor } from '../lib/format'
import { limitBarClass, portfolioLimit } from '../lib/limits'
import { VAR_METHOD_LABEL, headlineReport } from '../lib/var'
import { HoverTable } from './HoverTable'

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
    label: 'Vega ($/vol pt)',
    limitKind: 'portfolio_vega',
    tooltip: 'P&L from a 1 vol point rise in implied volatility.',
    value: (t) => t.vega,
    format: formatCompactCurrency,
  },
  {
    key: 'theta',
    label: 'Theta ($/day)',
    limitKind: 'portfolio_theta_floor',
    tooltip: 'P&L from one calendar day passing, all else equal.',
    value: (t) => t.theta,
    format: formatCompactCurrency,
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
      <span className="whitespace-nowrap text-xs uppercase tracking-wide text-neutral">{def.label}</span>
      <span className={`whitespace-nowrap text-lg font-semibold tabular-nums ${signColor(value)}`}>
        {def.format(value)}
      </span>
      {limit && (
        <>
          <div className="h-1.5 w-full overflow-hidden rounded bg-surface">
            <div
              className={`h-full ${limitBarClass(limit.state)}`}
              style={{ width: `${Math.min(limit.utilisation, 1) * 100}%` }}
            />
          </div>
          <span className="whitespace-nowrap text-xs text-neutral">
            {formatPercent(limit.utilisation)} of {def.format(limit.limit)}
          </span>
        </>
      )}
    </div>
  )
}

const VAR_METHOD_ORDER: VarMethod[] = ['delta_normal', 'delta_gamma_mc', 'historical']

/** The three methods' VaR95/VaR99/ES97.5 (Part 18), shown in a `VarTile`'s hover
 * panel, with the headline method's row picked out. */
function VarHoverContent({ varReport }: { varReport: VarReport }): ReactElement {
  const rows: Array<{ method: VarMethod; report: VarMethodReport }> = VAR_METHOD_ORDER.map(
    (method) => ({
      method,
      report:
        method === 'delta_normal'
          ? varReport.delta_normal
          : method === 'delta_gamma_mc'
            ? varReport.delta_gamma_mc
            : varReport.historical,
    }),
  )
  return (
    <table className="text-xs" data-testid="var-hover-table">
      <thead>
        <tr className="text-neutral">
          <th className="pr-2 text-left font-normal">Method</th>
          <th className="px-2 text-right font-normal">VaR 95%</th>
          <th className="px-2 text-right font-normal">VaR 99%</th>
          <th className="px-2 text-right font-normal">ES 97.5%</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(({ method, report }) => (
          <tr
            key={method}
            className={method === varReport.headline_method ? 'font-semibold' : ''}
          >
            <td className="whitespace-nowrap pr-2">{VAR_METHOD_LABEL[method]}</td>
            <td className="whitespace-nowrap px-2 text-right tabular-nums">
              {formatCompactCurrency(report.var_95)}
            </td>
            <td className="whitespace-nowrap px-2 text-right tabular-nums">
              {formatCompactCurrency(report.var_99)}
            </td>
            <td className="whitespace-nowrap px-2 text-right tabular-nums">
              {formatCompactCurrency(report.es_97_5)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/** The VaR tile: the headline method's VaR99 with a limit bar, and a hover table of
 * all three methods (Part 18). A `null` `varReport` (before the first snapshot)
 * renders a placeholder value with no bar and no hover content. */
function VarTile({
  varReport,
  limits,
}: {
  varReport: VarReport | null
  limits: LimitStatus[]
}): ReactElement {
  const limit = portfolioLimit(limits, 'portfolio_var99')
  const headline = varReport === null ? null : headlineReport(varReport)
  return (
    <div
      className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
      data-testid="tile-var"
      data-state={limit?.state ?? 'ok'}
    >
      <span className="whitespace-nowrap text-xs uppercase tracking-wide text-neutral">
        VaR 99% {varReport && `(${VAR_METHOD_LABEL[varReport.headline_method]})`}
      </span>
      <HoverTable
        trigger={
          <span
            className="whitespace-nowrap text-lg font-semibold tabular-nums"
            tabIndex={varReport === null ? undefined : 0}
          >
            {headline === null ? '—' : formatCompactCurrency(headline.var_99)}
          </span>
        }
      >
        {varReport && <VarHoverContent varReport={varReport} />}
      </HoverTable>
      {limit && (
        <>
          <div className="h-1.5 w-full overflow-hidden rounded bg-surface">
            <div
              className={`h-full ${limitBarClass(limit.state)}`}
              style={{ width: `${Math.min(limit.utilisation, 1) * 100}%` }}
            />
          </div>
          <span className="whitespace-nowrap text-xs text-neutral">
            {formatPercent(limit.utilisation)} of {formatCompactCurrency(limit.limit)}
          </span>
        </>
      )}
    </div>
  )
}

/** The risk tiles panel: net dollar delta, dollar gamma 1%, vega, theta, gross
 * exposure, headline VaR99 (Part 18), today's P&L and `compute_micros`, each
 * limited tile with a utilisation bar. Reads only its props, never the store
 * directly (see `PositionsTable`'s doc comment for why). `totals`/`limits`/
 * `pnlToday`/`computeMicros`/`varReport` are `null` before the first snapshot or
 * poll response. */
export function RiskTiles({
  totals,
  limits,
  pnlToday,
  computeMicros,
  varReport,
  className = '',
}: {
  totals: PortfolioTotals | null
  limits: LimitStatus[]
  pnlToday: number | null
  computeMicros: number | null
  varReport: VarReport | null
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
            <VarTile varReport={varReport} limits={limits} />
            <div
              className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
              title="Today's session P&L from full revaluation."
              data-testid="tile-pnl-today"
            >
              <span className="text-xs uppercase tracking-wide text-neutral">Today&apos;s P&amp;L</span>
              <span className={`whitespace-nowrap text-lg font-semibold tabular-nums ${signColor(pnlToday ?? 0)}`}>
                {pnlToday === null ? '—' : formatCompactCurrency(pnlToday)}
              </span>
            </div>
            <div
              className="flex flex-col gap-1 rounded-lg border border-border bg-surface-raised p-3"
              title="Microseconds spent computing the last tick."
              data-testid="tile-compute-micros"
            >
              <span className="text-xs uppercase tracking-wide text-neutral">Compute time</span>
              <span className="whitespace-nowrap text-lg font-semibold tabular-nums">
                {computeMicros === null ? '—' : formatMicros(computeMicros)}
              </span>
            </div>
          </>
        )}
      </div>
    </section>
  )
}
