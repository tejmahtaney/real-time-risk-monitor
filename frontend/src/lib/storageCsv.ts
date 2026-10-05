/** CSV text for the History page's two exports. Numbers are written in full. */
import type { DailyPnlRow } from '../api/generated/DailyPnlRow'
import type { HistoryPoint } from '../api/generated/HistoryPoint'

const DAILY_PNL_COLUMNS: ReadonlyArray<keyof DailyPnlRow> = [
  'session_date',
  'delta',
  'gamma',
  'vega',
  'theta',
  'rho',
  'higher_order',
  'dividends',
  'residual',
  'actual',
]

/** A session's daily P&L as CSV: a header, then one line per day. */
export function dailyPnlToCsv(rows: DailyPnlRow[]): string {
  const lines = [DAILY_PNL_COLUMNS.join(',')]
  for (const row of rows) lines.push(DAILY_PNL_COLUMNS.map((c) => String(row[c])).join(','))
  return `${lines.join('\n')}\n`
}

const HISTORY_COLUMNS = [
  'seq',
  'sim_time',
  'dollar_delta',
  'dollar_gamma_1pct',
  'vega',
  'theta',
  'gross_exposure',
  'market_value',
  'pnl_today',
  'worst_limit_utilisation',
  'var_99',
] as const

/** A history range as CSV. `pnl_today` is the day's actual P&L; a missing VaR is left blank. */
export function historyToCsv(points: HistoryPoint[]): string {
  const lines = [HISTORY_COLUMNS.join(',')]
  for (const p of points) {
    lines.push(
      [
        p.seq,
        p.sim_time,
        p.dollar_delta,
        p.dollar_gamma_1pct,
        p.vega,
        p.theta,
        p.gross_exposure,
        p.market_value,
        p.pnl_today.actual,
        p.worst_limit_utilisation,
        p.var_99 ?? '',
      ].join(','),
    )
  }
  return `${lines.join('\n')}\n`
}
