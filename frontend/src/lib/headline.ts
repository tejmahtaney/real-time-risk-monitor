/**
 * Builds the plain-English P&L headline sentence above the attribution chart, from
 * today's buckets only. A pure function, kept dependency-free (no store, no React)
 * so it is trivial to unit test with plain `PnlBuckets` fixtures.
 */
import type { PnlBuckets } from '../api/generated/PnlBuckets'
import { formatCompactCurrency } from './format'

/** The buckets the headline draws from: every Taylor-decomposition bucket except
 * `residual` and `actual`, per the prompt ("excluding residual and actual"). Order
 * doubles as the tie-break: when two buckets are exactly equal, the earlier one in
 * this list is named, so the sentence is deterministic. */
const HEADLINE_BUCKETS: Array<{ key: keyof PnlBuckets; label: string }> = [
  { key: 'delta', label: 'delta' },
  { key: 'gamma', label: 'gamma' },
  { key: 'vega', label: 'vega' },
  { key: 'theta', label: 'theta' },
  { key: 'rho', label: 'rho' },
  { key: 'higher_order', label: 'higher-order effects' },
  { key: 'dividends', label: 'dividends' },
]

/** Signed currency with an explicit `+` on a positive value (`formatCompactCurrency`
 * itself only ever adds the `−` sign), for the headline's "Net ..." clause. Exactly
 * zero gets no sign at all. */
function formatNet(value: number): string {
  const magnitude = formatCompactCurrency(Math.abs(value))
  if (!Number.isFinite(value) || value === 0) return magnitude
  return value > 0 ? `+${magnitude}` : `−${magnitude}`
}

/**
 * Builds today's headline sentence from `buckets`, such as:
 *
 * > Today you made $2.1k from theta and lost $5.0k to vega. Net +$1.3k.
 *
 * Net is the sum of the seven named buckets (`delta` + `gamma` + `vega` + `theta` +
 * `rho` + `higher_order` + `dividends`, i.e. `PnlBuckets::explained()`), not `actual` — the
 * headline is built from, and only from, the buckets it names, per the prompt.
 *
 * A day with no losing bucket omits the "lost ... to ..." clause (and vice versa
 * for no gaining bucket); a day where every named bucket is exactly zero gets its
 * own flat-day sentence instead of naming a "biggest" mover that isn't one.
 */
export function buildHeadline(buckets: PnlBuckets): string {
  const entries = HEADLINE_BUCKETS.map(({ key, label }) => ({ label, value: buckets[key] }))
  const net = entries.reduce((sum, e) => sum + e.value, 0)

  const gains = entries.filter((e) => e.value > 0)
  const losses = entries.filter((e) => e.value < 0)

  if (gains.length === 0 && losses.length === 0) {
    return `Today's P&L was flat across delta, gamma, vega, theta, rho, higher-order effects and dividends. Net ${formatNet(net)}.`
  }

  const clauses: string[] = []
  if (gains.length > 0) {
    const biggestGain = gains.reduce((best, e) => (e.value > best.value ? e : best))
    clauses.push(`made ${formatCompactCurrency(biggestGain.value)} from ${biggestGain.label}`)
  }
  if (losses.length > 0) {
    const biggestLoss = losses.reduce((worst, e) => (e.value < worst.value ? e : worst))
    clauses.push(`lost ${formatCompactCurrency(Math.abs(biggestLoss.value))} to ${biggestLoss.label}`)
  }

  return `Today you ${clauses.join(' and ')}. Net ${formatNet(net)}.`
}
