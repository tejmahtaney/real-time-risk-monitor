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

/** `$ per vol pt`: a vega figure, already in reporting units (dollars per vol point). */
export function formatPerVolPoint(value: number): string {
  return `${formatCompactCurrency(value)} / vol pt`
}

/** `$ per day`: a theta figure, already in reporting units (dollars per calendar day). */
export function formatPerDay(value: number): string {
  return `${formatCompactCurrency(value)} / day`
}

/** A plain compact number (no currency sign), such as `2.1k`, for share counts. */
export function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return '—'
  const negative = value < 0
  const magnitude = formatCompactMagnitude(Math.abs(value))
  return negative ? `−${magnitude}` : magnitude
}
