/** How full the storage folder is, for the usage bar's colour. */

/** `ok` below the warning level, `warning` from it up to the limit, `over` above the limit. */
export type UsageLevel = 'ok' | 'warning' | 'over'

/** The level for `used` bytes against `limit` bytes, with the warning at `warnFraction` of the
 * limit. Exactly at the warning level is a warning; exactly at the limit is still a warning. */
export function usageLevel(used: number, limit: number, warnFraction: number): UsageLevel {
  if (limit <= 0) return 'ok'
  if (used > limit) return 'over'
  return used >= limit * warnFraction ? 'warning' : 'ok'
}

/** Bytes as `1.5 MB`. Binary units, since the limit is set in them. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}
