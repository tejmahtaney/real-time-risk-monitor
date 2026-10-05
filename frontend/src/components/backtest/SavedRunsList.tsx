import { useState, type ReactElement } from 'react'

import type { BacktestListItem } from '../../api/generated/BacktestListItem'
import type { BacktestRunState } from '../../api/generated/BacktestRunState'
import { formatCompactCurrency, signColor } from '../../lib/format'

/** A run's source as one short label: the upload's file name, or "Archive". */
function sourceLabel(item: BacktestListItem): string {
  return item.source.kind === 'archive' ? 'Archive' : (item.source.filename ?? 'Upload')
}

const STATE_LABEL: Record<BacktestRunState, string> = {
  finished: 'Finished',
  unfinished: 'Unfinished',
  paused: 'In memory (paused)',
  playing: 'In memory (playing)',
  running_to_end: 'In memory (running)',
}

function DeleteConfirm({
  id,
  onConfirm,
  onCancel,
  error,
}: {
  id: string
  onConfirm: () => void
  onCancel: () => void
  error: string | null
}): ReactElement {
  const [typed, setTyped] = useState('')
  return (
    <div className="flex flex-col gap-2 text-xs" data-testid="delete-confirm">
      <p>
        This deletes the run&apos;s saved files and cannot be undone. Type the run id{' '}
        <code className="rounded bg-surface px-1">{id}</code> to confirm.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <input
          aria-label="Type the run id to confirm"
          value={typed}
          onChange={(event) => setTyped(event.target.value)}
          className="rounded border border-border bg-surface px-2 py-1"
        />
        <button
          type="button"
          disabled={typed !== id}
          onClick={onConfirm}
          className="rounded bg-down px-2 py-1 text-surface disabled:opacity-50"
        >
          Delete files
        </button>
        <button type="button" onClick={onCancel} className="rounded border border-border px-2 py-1">
          Cancel
        </button>
      </div>
      {error && <p className="text-down">{error}</p>}
    </div>
  )
}

/** The saved runs, newest first. Each row shows source, dates, state and total P&L, and can be
 * opened or deleted. A delete asks for the run id to be typed back before it enables. */
export function SavedRunsList({
  runs,
  onOpen,
  onDelete,
  onCompare,
}: {
  runs: BacktestListItem[]
  onOpen: (id: string) => void
  onDelete: (id: string) => Promise<void>
  /** Called with two finished run ids to compare. Without it the list has no compare control. */
  onCompare?: (a: string, b: string) => void
}): ReactElement {
  const [picked, setPicked] = useState<string[]>([])
  const [confirming, setConfirming] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const newestFirst = [...runs].sort((a, b) => b.created.localeCompare(a.created))

  const remove = async (id: string): Promise<void> => {
    setError(null)
    try {
      await onDelete(id)
      setConfirming(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'delete failed')
    }
  }

  const toggle = (id: string): void =>
    setPicked((current) =>
      current.includes(id) ? current.filter((x) => x !== id) : current.length < 2 ? [...current, id] : current,
    )
  const [first, second] = picked

  return (
    <section
      aria-label="Saved runs"
      className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Saved runs</h2>
        {onCompare && (
          <button
            type="button"
            disabled={first === undefined || second === undefined}
            onClick={() => first !== undefined && second !== undefined && onCompare(first, second)}
            className="rounded border border-border px-2 py-1 text-xs hover:bg-surface disabled:opacity-50"
          >
            Compare selected ({picked.length}/2)
          </button>
        )}
      </div>
      {newestFirst.length === 0 ? (
        <p className="text-sm text-neutral">No saved runs yet.</p>
      ) : (
        <ul className="flex flex-col divide-y divide-border">
          {newestFirst.map((run) => (
            <li
              key={run.id}
              className="flex flex-col gap-2 py-2"
              data-testid={`saved-run-${run.id}`}
            >
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm tabular-nums">
                {onCompare && (
                  <input
                    type="checkbox"
                    aria-label={`Compare ${run.id}`}
                    title={run.state === 'finished' ? undefined : 'Only finished runs can be compared'}
                    checked={picked.includes(run.id)}
                    disabled={run.state !== 'finished' || (!picked.includes(run.id) && picked.length >= 2)}
                    onChange={() => toggle(run.id)}
                  />
                )}
                <span className="font-mono text-xs">{run.id}</span>
                <span>{sourceLabel(run)}</span>
                <span className="text-neutral">
                  {run.start_date} to {run.last_date}
                </span>
                <span className="text-neutral">{run.days} days</span>
                <span
                  className={
                    run.state === 'unfinished'
                      ? 'rounded bg-warning/20 px-1.5 py-0.5 text-xs text-warning'
                      : 'text-xs text-neutral'
                  }
                >
                  {STATE_LABEL[run.state]}
                </span>
                <span
                  className={run.total_pnl === null ? 'text-neutral' : signColor(run.total_pnl)}
                >
                  {run.total_pnl === null ? '—' : formatCompactCurrency(run.total_pnl)}
                </span>
                <span className="ml-auto flex gap-2">
                  <button
                    type="button"
                    onClick={() => onOpen(run.id)}
                    className="rounded border border-border px-2 py-1 text-xs hover:bg-surface"
                  >
                    Open
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setError(null)
                      setConfirming(run.id)
                    }}
                    className="rounded border border-border px-2 py-1 text-xs text-down hover:bg-surface"
                  >
                    Delete
                  </button>
                </span>
              </div>
              {confirming === run.id && (
                <DeleteConfirm
                  id={run.id}
                  error={error}
                  onCancel={() => setConfirming(null)}
                  onConfirm={() => void remove(run.id)}
                />
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
