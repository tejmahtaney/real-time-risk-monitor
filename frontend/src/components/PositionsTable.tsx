import { useEffect, useRef, useState, type ReactElement } from 'react'

import type { PositionRisk } from '../api/generated/PositionRisk'
import { api, ApiError } from '../api/rest'
import { formatCompactCurrency, formatCompactNumber, signColor } from '../lib/format'
import {
  groupPositions,
  loadSortState,
  saveSortState,
  sortRows,
  type SortColumn,
  type SortState,
} from '../lib/positionGroups'
import { symbolColor } from '../lib/symbolColors'

interface Column {
  key: SortColumn
  label: string
  align?: 'left' | 'right'
}

const COLUMNS: Column[] = [
  { key: 'description', label: 'Description', align: 'left' },
  { key: 'quantity', label: 'Qty' },
  { key: 'spot', label: 'Spot' },
  { key: 'mark', label: 'Mark' },
  { key: 'market_value', label: 'Mkt value' },
  { key: 'unrealised_pnl', label: 'Unreal. P&L' },
  { key: 'share_delta', label: 'Share Δ' },
  { key: 'dollar_delta', label: '$ Δ' },
  { key: 'share_gamma', label: 'Share Γ' },
  { key: 'vega', label: 'Vega' },
  { key: 'theta', label: 'Theta/day' },
  { key: 'status', label: 'Status' },
]

const DEFAULT_SORT: SortState = { column: 'description', direction: 'asc' }

function numberCell(value: number, formatter: (v: number) => string = formatCompactCurrency): string {
  return formatter(value)
}

/** A flat key for one cell, used to remember its previous value across renders
 * so a genuine change can pulse green or red. */
function cellKey(rowId: string, column: SortColumn): string {
  return `${rowId}:${column}`
}

const NUMERIC_COLUMNS: SortColumn[] = [
  'quantity',
  'spot',
  'mark',
  'market_value',
  'unrealised_pnl',
  'share_delta',
  'dollar_delta',
  'share_gamma',
  'vega',
  'theta',
]

function cellValue(row: PositionRisk, column: SortColumn): number | string {
  switch (column) {
    case 'description':
      return row.description
    case 'quantity':
      return row.quantity
    case 'spot':
      return row.spot
    case 'mark':
      return row.mark
    case 'market_value':
      return row.market_value
    case 'unrealised_pnl':
      return row.unrealised_pnl
    case 'share_delta':
      return row.reporting.share_delta
    case 'dollar_delta':
      return row.reporting.dollar_delta
    case 'share_gamma':
      return row.reporting.share_gamma
    case 'vega':
      return row.reporting.vega
    case 'theta':
      return row.reporting.theta
    case 'status':
      return row.status.state
  }
}

/** The exact text a numeric cell renders, using the same formatter its `<td>` does
 * below — {@link useCellFlashes} diffs this, not the raw number, so a change too
 * small to show up on screen (spot moving a fraction of a cent, say) does not
 * flash a cell whose displayed text hasn't actually changed. */
function cellText(row: PositionRisk, column: SortColumn): string {
  switch (column) {
    case 'quantity':
    case 'share_delta':
    case 'share_gamma':
      return formatCompactNumber(cellValue(row, column) as number)
    default:
      return formatCompactCurrency(cellValue(row, column) as number)
  }
}

/** Tracks every visible cell's previously displayed text and raw value, and
 * returns a lookup of which cells should pulse, and which direction, on this
 * render. A cell pulses only when its *displayed text* changes (see {@link
 * cellText}); the direction shown still comes from the raw value, so a pulse
 * always points the right way even when the rounded text alone wouldn't say.
 * Cleared out again after {@link PULSE_MS}. */
const PULSE_MS = 400

function useCellFlashes(rows: PositionRisk[]): Map<string, 'up' | 'down'> {
  const previousText = useRef<Map<string, string>>(new Map())
  const previousValue = useRef<Map<string, number>>(new Map())
  const [flashes, setFlashes] = useState<Map<string, 'up' | 'down'>>(new Map())

  useEffect(() => {
    const next = new Map<string, 'up' | 'down'>()
    for (const row of rows) {
      for (const column of NUMERIC_COLUMNS) {
        const key = cellKey(row.id, column)
        const text = cellText(row, column)
        const value = cellValue(row, column) as number
        const priorText = previousText.current.get(key)
        const priorValue = previousValue.current.get(key)
        if (priorText !== undefined && text !== priorText) {
          const direction: 'up' | 'down' =
            Number.isFinite(value) && Number.isFinite(priorValue) && value < (priorValue as number)
              ? 'down'
              : 'up'
          next.set(key, direction)
        }
        previousText.current.set(key, text)
        previousValue.current.set(key, value)
      }
    }
    if (next.size > 0) {
      setFlashes(next)
      const timer = setTimeout(() => setFlashes(new Map()), PULSE_MS)
      return () => clearTimeout(timer)
    }
    return undefined
  }, [rows])

  return flashes
}

function RowMenu({ position, onRemoved }: { position: PositionRisk; onRemoved: () => void }): ReactElement {
  const [open, setOpen] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const remove = async (): Promise<void> => {
    setPending(true)
    setError(null)
    try {
      await api.removePosition(position.id)
      onRemoved()
      setOpen(false)
      setConfirming(false)
    } catch (err) {
      setError(err instanceof ApiError ? err.message : 'failed to remove position')
    } finally {
      setPending(false)
    }
  }

  return (
    <div className="relative inline-block text-left">
      <button
        type="button"
        aria-label={`Row menu for ${position.description}`}
        className="rounded px-1.5 py-0.5 text-neutral hover:bg-surface"
        onClick={() => {
          setOpen((v) => !v)
          setConfirming(false)
          setError(null)
        }}
      >
        ⋮
      </button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 z-10 mt-1 w-56 rounded border border-border bg-surface-raised p-2 text-left text-xs shadow-lg"
        >
          {!confirming ? (
            <button
              type="button"
              role="menuitem"
              className="block w-full rounded px-2 py-1 text-left text-down hover:bg-surface"
              onClick={() => setConfirming(true)}
            >
              Remove position
            </button>
          ) : (
            <div className="flex flex-col gap-2 px-2 py-1">
              <p>Remove {position.description}?</p>
              {error && <p className="text-down">{error}</p>}
              <div className="flex gap-2">
                <button
                  type="button"
                  disabled={pending}
                  className="rounded bg-down px-2 py-1 text-surface disabled:opacity-50"
                  onClick={() => void remove()}
                >
                  {pending ? 'Removing…' : 'Confirm'}
                </button>
                <button
                  type="button"
                  className="rounded border border-border px-2 py-1"
                  onClick={() => setConfirming(false)}
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** The positions table: grouped by underlying with a dollar-measure subtotal per
 * group, sortable within each group, sticky header, right-aligned tabular
 * numbers, per-cell flash on change, and a row menu to remove a position. Reads
 * only its props, never the store directly, so Part 21's backtest replay can
 * feed it too (see the file's own comment in `App.tsx`/`Dashboard.tsx`). */
export function PositionsTable({
  positions,
  onPositionsChanged,
  readOnly = false,
  className = '',
}: {
  positions: PositionRisk[]
  onPositionsChanged?: () => void
  /** Hides the row menu, so a replayed book cannot change the live one. */
  readOnly?: boolean
  className?: string
}): ReactElement {
  const [sort, setSort] = useState<SortState>(() => loadSortState() ?? DEFAULT_SORT)
  const flashes = useCellFlashes(positions)

  const setColumn = (column: SortColumn): void => {
    setSort((current) => {
      const next: SortState =
        current.column === column
          ? { column, direction: current.direction === 'asc' ? 'desc' : 'asc' }
          : { column, direction: 'asc' }
      saveSortState(next)
      return next
    })
  }

  const groups = groupPositions(positions)

  return (
    <section
      className={`flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="Positions"
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Positions</h2>
      <div className="overflow-x-auto">
        <table className="w-full min-w-max border-collapse text-sm tabular-nums">
          <thead className="sticky top-0 bg-surface-raised">
            <tr>
              {COLUMNS.map((column) => (
                <th
                  key={column.key}
                  scope="col"
                  className={`cursor-pointer select-none whitespace-nowrap border-b border-border px-2 py-1 font-medium text-neutral ${
                    column.align === 'left' ? 'text-left' : 'text-right'
                  }`}
                  onClick={() => setColumn(column.key)}
                  aria-sort={
                    sort.column === column.key
                      ? sort.direction === 'asc'
                        ? 'ascending'
                        : 'descending'
                      : 'none'
                  }
                >
                  {column.label}
                  {sort.column === column.key && (sort.direction === 'asc' ? ' ▲' : ' ▼')}
                </th>
              ))}
              <th scope="col" className="border-b border-border px-2 py-1" />
            </tr>
          </thead>
          <tbody>
            {groups.length === 0 && (
              <tr>
                <td colSpan={COLUMNS.length + 1} className="px-2 py-4 text-center text-neutral">
                  No positions.
                </td>
              </tr>
            )}
            {groups.map((group) => (
              <GroupRows
                key={group.underlying}
                group={group}
                sort={sort}
                flashes={flashes}
                onRemoved={readOnly ? undefined : () => onPositionsChanged?.()}
              />
            ))}
          </tbody>
        </table>
      </div>
    </section>
  )
}

function GroupRows({
  group,
  sort,
  flashes,
  onRemoved,
}: {
  group: ReturnType<typeof groupPositions>[number]
  sort: SortState
  flashes: Map<string, 'up' | 'down'>
  onRemoved: (() => void) | undefined
}): ReactElement {
  const rows = sortRows(group.rows, sort)
  return (
    <>
      <tr>
        <td colSpan={COLUMNS.length + 1} className="bg-surface px-2 py-1 text-xs font-semibold text-neutral">
          <span className="flex items-center gap-1.5">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ backgroundColor: symbolColor(group.underlying) }}
              aria-hidden
            />
            {group.underlying}
          </span>
        </td>
      </tr>
      {rows.map((row) => (
        <PositionRow key={row.id} row={row} flashes={flashes} onRemoved={onRemoved} />
      ))}
      <tr className="border-t border-border font-medium">
        <td className="whitespace-nowrap px-2 py-1">Subtotal</td>
        <td className="whitespace-nowrap px-2 py-1 text-right text-neutral" />
        <td className="whitespace-nowrap px-2 py-1 text-right text-neutral" />
        <td className="whitespace-nowrap px-2 py-1 text-right text-neutral" />
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.market_value)}`}>
          {numberCell(group.subtotal.market_value)}
        </td>
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.unrealised_pnl)}`}>
          {numberCell(group.subtotal.unrealised_pnl)}
        </td>
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.share_delta)}`}>
          {numberCell(group.subtotal.share_delta, formatCompactNumber)}
        </td>
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.dollar_delta)}`}>
          {numberCell(group.subtotal.dollar_delta)}
        </td>
        <td
          className="whitespace-nowrap px-2 py-1 text-right text-neutral"
          title="Share gamma is not shown on a subtotal; it is meaningful per underlying, and the per-position figures above already give it."
        >
          —
        </td>
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.vega)}`}>
          {numberCell(group.subtotal.vega)}
        </td>
        <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(group.subtotal.theta)}`}>
          {numberCell(group.subtotal.theta)}
        </td>
        <td className="px-2 py-1" />
        <td className="px-2 py-1" />
      </tr>
    </>
  )
}

function PositionRow({
  row,
  flashes,
  onRemoved,
}: {
  row: PositionRisk
  flashes: Map<string, 'up' | 'down'>
  onRemoved: (() => void) | undefined
}): ReactElement {
  const expired = row.status.state === 'expired'
  const errorReason = row.status.state === 'error' ? row.status.reason : undefined
  const pulseClass = (column: SortColumn): string => {
    const pulse = flashes.get(cellKey(row.id, column))
    return pulse ? `text-pulse-${pulse}` : ''
  }

  return (
    <tr
      className={expired ? 'text-neutral/60' : ''}
      title={errorReason}
      data-testid="position-row"
    >
      <td className="whitespace-nowrap px-2 py-1 text-left">
        {row.description}
        {row.adjusted && (
          <span
            className="ml-2 rounded border border-warning/60 px-1 text-[10px] font-semibold uppercase text-warning"
            title="A corporate action adjusted this contract's strike or multiplier"
            data-testid="adj-badge"
          >
            adj
          </span>
        )}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${pulseClass('quantity')}`}>
        {formatCompactNumber(row.quantity)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${pulseClass('spot')}`}>
        {formatCompactCurrency(row.spot)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${pulseClass('mark')}`}>
        {formatCompactCurrency(row.mark)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row.market_value)} ${pulseClass('market_value')}`}>
        {formatCompactCurrency(row.market_value)}
      </td>
      <td
        className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row.unrealised_pnl)} ${pulseClass('unrealised_pnl')}`}
      >
        {formatCompactCurrency(row.unrealised_pnl)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${pulseClass('share_delta')}`}>
        {formatCompactNumber(row.reporting.share_delta)}
      </td>
      <td
        className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row.reporting.dollar_delta)} ${pulseClass('dollar_delta')}`}
      >
        {formatCompactCurrency(row.reporting.dollar_delta)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${pulseClass('share_gamma')}`}>
        {formatCompactNumber(row.reporting.share_gamma)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row.reporting.vega)} ${pulseClass('vega')}`}>
        {formatCompactCurrency(row.reporting.vega)}
      </td>
      <td className={`whitespace-nowrap px-2 py-1 text-right ${signColor(row.reporting.theta)} ${pulseClass('theta')}`}>
        {formatCompactCurrency(row.reporting.theta)}
      </td>
      <td className="whitespace-nowrap px-2 py-1 text-right capitalize">{row.status.state}</td>
      <td className="whitespace-nowrap px-2 py-1 text-right">
        {onRemoved && <RowMenu position={row} onRemoved={onRemoved} />}
      </td>
    </tr>
  )
}
