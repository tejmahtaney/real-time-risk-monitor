/** Helpers for `VarReport` (Part 18): picking the headline method's own sub-report,
 * and human labels for `VarMethod`. Kept separate from `riskStore.ts` so panels that
 * only need this logic (not the whole store) can import it without pulling the store
 * in — see `PositionsTable`'s doc comment for why panels avoid the store directly. */
import type { VarMethod } from '../api/generated/VarMethod'
import type { VarMethodReport } from '../api/generated/VarMethodReport'
import type { VarReport } from '../api/generated/VarReport'

/** The sub-report `varReport.headline_method` names — what the dashboard shows as
 * its headline figure and what `LimitKind`'s `portfolio_var99` is evaluated
 * against. Mirrors `risk::VarReport::headline` on the Rust side exactly; if that
 * changes, this must change with it. */
export function headlineReport(varReport: VarReport): VarMethodReport {
  switch (varReport.headline_method) {
    case 'delta_normal':
      return varReport.delta_normal
    case 'delta_gamma_mc':
      return varReport.delta_gamma_mc
    case 'historical':
      return varReport.historical
  }
}

/** `headlineReport(varReport).var_99` — the single scalar `HistoryPoint.var_99` and
 * the Greeks-over-time VaR chart both plot. */
export function headlineVar99(varReport: VarReport): number {
  return headlineReport(varReport).var_99
}

export const VAR_METHOD_LABEL: Record<VarMethod, string> = {
  delta_normal: 'Delta-normal',
  delta_gamma_mc: 'Delta-gamma Monte Carlo',
  historical: 'Historical simulation',
}
