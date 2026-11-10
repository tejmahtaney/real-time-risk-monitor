/**
 * Turns a portfolio-scoped `LimitStatus` into the reference line and shaded band the
 * Greeks-over-time charts draw: a dashed line at the limit, and a shaded band from
 * the warning threshold up to it.
 */
import type { LimitKind } from '../api/generated/LimitKind'
import type { LimitStatus } from '../api/generated/LimitStatus'

/**
 * `config/limits.toml`'s `warning_threshold`: one fraction, applied uniformly to
 * every limit (see `risk::LimitsConfig`). Not carried on `LimitStatus` itself (only
 * `utilisation`, `value`, `limit` and the already-computed `state` are), so it is
 * reproduced here rather than plumbed through a new API field, since this part's
 * prompt scopes the work to `frontend/`. If `config/limits.toml`'s value ever
 * changes, this constant must change with it.
 */
export const LIMIT_WARNING_THRESHOLD = 0.8

/** The five Greeks-over-time charts, by the reporting-unit field they plot. */
export type GreekMetric = 'dollar_delta' | 'dollar_gamma_1pct' | 'vega' | 'theta' | 'var_99'

const METRIC_LIMIT_KIND: Record<GreekMetric, LimitKind> = {
  dollar_delta: 'portfolio_dollar_delta',
  dollar_gamma_1pct: 'portfolio_dollar_gamma1_pct',
  vega: 'portfolio_vega',
  theta: 'portfolio_theta_floor',
  var_99: 'portfolio_var99',
}

/** One side of a chart's bound: the dashed line's value, and where its shaded band
 * starts, both in the metric's own signed units. */
export interface ChartBound {
  limit: number
  warningStart: number
}

export interface MetricBounds {
  /** The upper bound, or `null` when the metric has no upper limit (theta: a book
   * earning theta cannot breach the floor, so there is nothing to cap above). */
  upper: ChartBound | null
  /** The lower bound, or `null` when there is none configured. */
  lower: ChartBound | null
}

const NO_BOUNDS: MetricBounds = { upper: null, lower: null }

/**
 * Computes the reference line(s) and band(s) for `metric` from the portfolio-scoped
 * limits in a snapshot's `limits` array.
 *
 * `portfolio_dollar_delta`, `portfolio_dollar_gamma1_pct` and `portfolio_vega` are
 * all limits on an absolute value (`|value| / limit`), so a breach can come from
 * either side: both an upper and a lower (mirrored) bound are returned.
 *
 * `portfolio_theta_floor` compares `max(0, -theta)` against its limit (see
 * `PortfolioLimits::theta_floor`'s doc comment) — positive theta can never breach
 * it, so only a lower bound (very negative theta) makes sense; `upper` is `null`.
 *
 * `portfolio_var99` (Part 18) is the mirror image of theta: VaR is always reported
 * as a non-negative number (never an absolute value of something that can go either
 * way), so only an upper bound makes sense; `lower` is `null`.
 */
export function metricBounds(limits: LimitStatus[], metric: GreekMetric): MetricBounds {
  const status = limits.find(
    (l) => l.name === METRIC_LIMIT_KIND[metric] && l.scope.kind === 'portfolio',
  )
  if (!status || !Number.isFinite(status.limit) || status.limit <= 0) return NO_BOUNDS

  const warningMagnitude = status.limit * LIMIT_WARNING_THRESHOLD
  const upper: ChartBound = { limit: status.limit, warningStart: warningMagnitude }

  if (metric === 'theta') {
    const lower: ChartBound = { limit: -status.limit, warningStart: -warningMagnitude }
    return { upper: null, lower }
  }
  if (metric === 'var_99') {
    return { upper, lower: null }
  }
  const lower: ChartBound = { limit: -status.limit, warningStart: -warningMagnitude }
  return { upper, lower }
}

/** The smallest half-width a padded domain is ever given, so a series flat at
 * exactly 0 does not collapse to a zero-height axis. */
const MIN_HALF_SPAN = 1

/** How far beyond the data-only domain a configured limit is still allowed to pull
 * the axis out to. Past this, drawing the line/band would flatten the chart the
 * same way anchoring the whole axis at the limit always did — this book's limits
 * are tuned to sit only 30–50% above the data's own observed peak in ordinary
 * conditions, so 1.5× only excludes a limit
 * that is genuinely far from what the chart is actually showing. */
const LIMIT_VISIBILITY_FACTOR = 1.5

/** How close the data's own extreme must come to the warning line, as a fraction of
 * the warning magnitude, before the line and band are brought into view. Below this
 * the band is only a corner label, so a value sitting at 65–80% of its limit does
 * not stretch the axis to include a band it has not reached. */
const WARNING_APPROACH_MARGIN = 0.1

/** The least share of the plot height the data-only domain may take once the axis is
 * extended out to a limit. Below it the band is dropped to a corner label. */
const MIN_DATA_FILL = 0.5

/** What a Greeks-over-time chart needs to draw one metric: the y-axis domain, the
 * one reference bound to draw (if any), and a limit value to show as a small
 * corner label instead, when the real limit is configured but too far away to
 * draw without flattening the chart. */
export interface ChartScale {
  domain: [number, number]
  /** The bound to draw as a dashed line and shaded band, or `null` when either no
   * limit is configured for the data's side, or the configured one is too far away
   * (see {@link ChartScale.offscreenLimit}). */
  visibleBound: ChartBound | null
  /** A configured limit that sits too far outside the data's own range to draw —
   * shown as a small "limit $X" label instead of stretching the axis to fit it. */
  offscreenLimit: number | null
}

/**
 * Computes a Greeks-over-time chart's y-axis domain from `values` alone — about
 * 10% padding beyond the data's own min/max, never from a configured limit —
 * because `portfolio_dollar_delta`/`_gamma1_pct`/`_vega` are all limits on an
 * *absolute* value, so both `bounds.upper` and `bounds.lower` exist together and
 * are usually both far from zero (e.g. ±$9.5k for gamma); including both in the
 * domain the way an earlier version of this function did stretches the axis to
 * ±the limit regardless of where the data actually sits, flattening the chart
 * exactly the same way Recharts' own anchor-at-0 default did.
 *
 * Only one side's bound is ever drawn — whichever side the data's own extremes
 * lean towards (by magnitude; a tie leans positive) — since a chart cannot
 * meaningfully be "near breach" on a side its data isn't near. That bound is drawn
 * only if it sits within {@link LIMIT_VISIBILITY_FACTOR}× the data-only domain's
 * reach on that side; otherwise it is surfaced as {@link ChartScale.offscreenLimit}
 * instead of pulling the axis out to it.
 */
export function computeChartScale(values: number[], bounds: MetricBounds): ChartScale {
  const finite = values.filter((v) => Number.isFinite(v))
  if (finite.length === 0) {
    return { domain: [-1, 1], visibleBound: null, offscreenLimit: null }
  }

  const dataMin = Math.min(...finite)
  const dataMax = Math.max(...finite)
  const positiveExtent = Math.max(dataMax, 0)
  const negativeExtent = Math.max(-dataMin, 0)
  const side: 'upper' | 'lower' = positiveExtent >= negativeExtent ? 'upper' : 'lower'
  const candidate = side === 'upper' ? bounds.upper : bounds.lower

  const range = dataMax - dataMin
  const pad = range > 0 ? range * 0.1 : dataMax === 0 ? MIN_HALF_SPAN : Math.abs(dataMax) * 0.1
  let domainMin = dataMin - pad
  let domainMax = dataMax + pad

  if (!candidate) {
    return { domain: [domainMin, domainMax], visibleBound: null, offscreenLimit: null }
  }

  const limitMagnitude = Math.abs(candidate.limit)
  const domainReach = side === 'upper' ? domainMax : Math.abs(domainMin)
  const dataExtent = side === 'upper' ? positiveExtent : negativeExtent
  const warningMagnitude = Math.abs(candidate.warningStart)
  const nearWarning = dataExtent >= warningMagnitude * (1 - WARNING_APPROACH_MARGIN)
  const withinReach = limitMagnitude <= domainReach * LIMIT_VISIBILITY_FACTOR

  // The axis the band would need, so the data's share of the plot height is known
  // before deciding to draw it.
  const extendedReach = Math.max(domainReach, limitMagnitude, warningMagnitude)
  const extendedEdge = extendedReach + extendedReach * 0.1
  const dataOnlySpan = domainMax - domainMin
  const extendedSpan = side === 'upper' ? extendedEdge - domainMin : domainMax + extendedEdge
  const fillsPlot = dataOnlySpan >= extendedSpan * MIN_DATA_FILL
  const visible = withinReach && nearWarning && fillsPlot

  if (!visible) {
    return { domain: [domainMin, domainMax], visibleBound: null, offscreenLimit: limitMagnitude }
  }

  // Extend only the active side out to the limit (plus its own 10% headroom), so
  // the line and band are never clipped; the other side keeps its data-only pad.
  if (side === 'upper') {
    domainMax = extendedEdge
  } else {
    domainMin = -extendedEdge
  }
  return { domain: [domainMin, domainMax], visibleBound: candidate, offscreenLimit: null }
}
