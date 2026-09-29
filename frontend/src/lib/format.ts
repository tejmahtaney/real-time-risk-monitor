/**
 * Formatting helpers for numeric display. Kept pure and dependency-free so they are
 * trivial to unit test: no locale surprises, no React.
 */

/** Signed colour class for a number: up (positive), down (negative), or neutral (zero). */
export function signColor(value: number): 'text-up' | 'text-down' | 'text-neutral' {
  if (!Number.isFinite(value) || value === 0) return 'text-neutral'
  return value > 0 ? 'text-up' : 'text-down'
}

/**
 * Compact signed currency, such as `$2.1k` or `−$5.0k`. Uses a real minus sign
 * (U+2212), not a hyphen, so it reads correctly next to the dollar sign. Negative
 * zero is treated as zero (no stray minus sign on a value that rounds to nothing).
 */
export function formatCompactCurrency(value: number): string {
  if (!Number.isFinite(value)) return '—'
  const normalised = value === 0 ? 0 : value
  const negative = normalised < 0
  const abs = Math.abs(normalised)
  const magnitude = formatCompactMagnitude(abs)
  return negative ? `−$${magnitude}` : `$${magnitude}`
}

/** The unsigned, compacted magnitude used by {@link formatCompactCurrency}. */
function formatCompactMagnitude(abs: number): string {
  const units: Array<[number, string]> = [
    [1e12, 'T'],
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'k'],
  ]
  for (const [threshold, suffix] of units) {
    if (abs >= threshold) {
      return `${(abs / threshold).toFixed(1)}${suffix}`
    }
  }
  if (abs >= 1) return abs.toFixed(abs < 10 ? 2 : 0)
  if (abs === 0) return '0.00'
  // Very small non-zero values: avoid printing "$0" for a real, tiny number.
  return abs.toPrecision(2)
}

/** A fraction (0 to 1, or beyond for a limit that's breached) as a percentage string. */
export function formatPercent(fraction: number, digits = 1): string {
  if (!Number.isFinite(fraction)) return '—'
  return `${(fraction * 100).toFixed(digits)}%`
}

/** A plain compact number (no currency sign), such as `2.1k`, for share counts. */
export function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return '—'
  const negative = value < 0
  const magnitude = formatCompactMagnitude(Math.abs(value))
  return negative ? `−${magnitude}` : magnitude
}

/** `compute_micros` as `123 µs` or, once it crosses a millisecond, `1.23 ms`. */
export function formatMicros(micros: number): string {
  if (!Number.isFinite(micros)) return '—'
  return micros >= 1_000 ? `${(micros / 1_000).toFixed(2)} ms` : `${micros.toFixed(0)} µs`
}

/** The market's own timezone (CLAUDE.md: "The market time zone is
 * America/New_York"). Every `sim_time` on the wire is a UTC instant with no
 * timezone of its own to display in — {@link formatSimTime} always renders it here,
 * explicitly, rather than in whatever timezone the viewer's browser happens to be
 * set to. */
const MARKET_TIME_ZONE = 'America/New_York'

/**
 * A `sim_time` ISO timestamp rendered in {@link MARKET_TIME_ZONE} with an explicit
 * "ET" suffix, so `10:16:00 ET` reads the same on a browser in New York and one in
 * Singapore — an unlabelled, unzoned `toLocaleTimeString` would show the Singapore
 * browser its own local wall clock (`10:16:00 PM`) for a 10:16 AM New York tick.
 * `seconds: false` drops the seconds field, for a compact axis tick; `withDate:
 * true` adds the month and day, for a context with no date otherwise visible. An
 * unparseable value (should not happen; every `sim_time` on the wire is a real
 * `DateTime<Utc>`) is returned as-is rather than throwing. */
export function formatSimTime(iso: string, opts: { seconds?: boolean; withDate?: boolean } = {}): string {
  const { seconds = true, withDate = false } = opts
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const formatted = date.toLocaleString('en-US', {
    timeZone: MARKET_TIME_ZONE,
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    ...(seconds ? { second: '2-digit' } : {}),
    ...(withDate ? { month: 'short', day: 'numeric' } : {}),
  })
  return `${formatted} ET`
}
