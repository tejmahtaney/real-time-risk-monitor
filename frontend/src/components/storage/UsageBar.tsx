import type { ReactElement } from 'react'

import type { StorageUsageResponse } from '../../api/generated/StorageUsageResponse'
import { formatBytes, usageLevel, type UsageLevel } from '../../lib/usage'

const FILL: Record<UsageLevel, string> = {
  ok: 'bg-info',
  warning: 'bg-warning',
  over: 'bg-breach',
}

/** Storage use as a bar against the limit, then one thinner bar per folder, scaled to the largest folder.
 * The fill turns amber at the warning level and red above the limit. The storage path is shown
 * under it. */
export function UsageBar({ usage }: { usage: StorageUsageResponse }): ReactElement {
  const level = usageLevel(usage.total_bytes, usage.limit_bytes, usage.warn_fraction)
  const share = (bytes: number): number =>
    usage.limit_bytes > 0 ? Math.min(100, (bytes / usage.limit_bytes) * 100) : 0
  const folders = Object.entries(usage.usage_bytes).sort((a, b) => b[1] - a[1])
  // Folder bars use the largest folder as full width, so small stores still show their split.
  const largest = folders[0]?.[1] ?? 0
  const folderShare = (bytes: number): number => (largest > 0 ? (bytes / largest) * 100 : 0)
  return (
    <section
      aria-label="Storage usage"
      className="flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4"
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Storage usage</h2>
      <div className="flex flex-col gap-1">
        <div className="flex justify-between text-sm tabular-nums">
          <span>
            {formatBytes(usage.total_bytes)} of {formatBytes(usage.limit_bytes)}
          </span>
          <span data-testid="usage-level" data-level={level} className="text-neutral">
            {level === 'ok'
              ? 'within limit'
              : level === 'warning'
                ? 'warning level reached'
                : 'over the limit'}
          </span>
        </div>
        <div
          role="progressbar"
          aria-label="Total storage used"
          aria-valuemin={0}
          aria-valuemax={usage.limit_bytes}
          aria-valuenow={usage.total_bytes}
          className="relative h-4 overflow-hidden rounded bg-surface"
        >
          <div
            data-testid="usage-fill"
            data-level={level}
            className={`h-full ${FILL[level]}`}
            style={{ width: `${share(usage.total_bytes)}%` }}
          />
          <div
            className="absolute inset-y-0 w-px bg-neutral"
            style={{ left: `${usage.warn_fraction * 100}%` }}
            title="Warning level"
          />
        </div>
      </div>
      <ul className="flex flex-col gap-1">
        {folders.map(([folder, bytes]) => (
          <li
            key={folder}
            className="grid grid-cols-[8rem_1fr_5rem] items-center gap-2 text-xs tabular-nums"
          >
            <span className="truncate">{folder}</span>
            <div className="h-2 overflow-hidden rounded bg-surface">
              <div
                data-testid="folder-fill"
                className={`h-full ${FILL[level]} opacity-70`}
                style={{ width: `${folderShare(bytes)}%` }}
              />
            </div>
            <span className="text-right text-neutral">{formatBytes(bytes)}</span>
          </li>
        ))}
      </ul>
      <p className="text-xs text-neutral">
        Stored in <code className="rounded bg-surface px-1">{usage.storage_path}</code>
      </p>
    </section>
  )
}
