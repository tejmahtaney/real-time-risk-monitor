/** Lookup and presentation helpers for `LimitStatus`, shared by the risk tiles
 * and the alerts pane. */
import type { LimitKind } from '../api/generated/LimitKind'
import type { LimitState } from '../api/generated/LimitState'
import type { LimitStatus } from '../api/generated/LimitStatus'
import type { Scope } from '../api/generated/Scope'

/** The portfolio-scope status for one limit kind, or `undefined` if it is not
 * (yet) present in `limits` — every configured limit is evaluated every tick,
 * so this is only `undefined` before the first snapshot arrives. */
export function portfolioLimit(limits: LimitStatus[], name: LimitKind): LimitStatus | undefined {
  return limits.find((l) => l.name === name && l.scope.kind === 'portfolio')
}

/** Ranks a limit state for sorting: breach first, then warning, then ok. */
export function stateRank(state: LimitState): number {
  return state === 'breach' ? 2 : state === 'warning' ? 1 : 0
}

const BAR_CLASS: Record<LimitState, string> = {
  ok: 'bg-up',
  warning: 'bg-warning',
  breach: 'bg-breach',
}

/** The utilisation bar's fill colour for a limit state. */
export function limitBarClass(state: LimitState): string {
  return BAR_CLASS[state]
}

const KIND_LABEL: Record<LimitKind, string> = {
  portfolio_dollar_delta: 'Portfolio $ delta',
  portfolio_dollar_gamma1_pct: 'Portfolio $ gamma 1%',
  portfolio_vega: 'Portfolio vega',
  portfolio_theta_floor: 'Portfolio theta floor',
  gross_exposure: 'Gross exposure',
  concentration: 'Concentration',
  underlying_dollar_delta: 'Underlying $ delta',
  portfolio_var99: 'Portfolio VaR 99%',
}

/** A human label for a `LimitKind`. */
export function limitLabel(kind: LimitKind): string {
  return KIND_LABEL[kind]
}

/** A human label for a `Scope`: "Portfolio" or the underlying's symbol. */
export function scopeLabel(scope: Scope): string {
  return scope.kind === 'portfolio' ? 'Portfolio' : scope.symbol
}
