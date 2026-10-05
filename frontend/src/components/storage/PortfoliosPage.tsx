import { useState, type ReactElement } from 'react'

import type { VersionSummary } from '../../api/generated/VersionSummary'
import { api } from '../../api/rest'
import { formatSimTime } from '../../lib/format'
import { diffBooks, entryFromRisk, entryFromSpec, type BookDiff } from '../../lib/portfolioDiff'
import { formatBytes } from '../../lib/usage'
import { PositionsTable } from '../PositionsTable'
import { useLoad } from './useLoad'

/** What loading a version would change, as three short lists. */
export function DiffSummary({ diff }: { diff: BookDiff }): ReactElement {
  if (diff.identical) {
    return <p className="text-sm text-neutral">This version matches the current book. Loading it changes nothing.</p>
  }
  return (
    <div className="flex flex-col gap-2 text-sm" data-testid="version-diff">
      {diff.added.length > 0 && (
        <div>
          <h4 className="font-medium text-up">Added ({diff.added.length})</h4>
          <ul>
            {diff.added.map((e) => (
              <li key={e.id}>
                {e.description} <span className="tabular-nums text-neutral">× {e.quantity}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {diff.removed.length > 0 && (
        <div>
          <h4 className="font-medium text-down">Removed ({diff.removed.length})</h4>
          <ul>
            {diff.removed.map((e) => (
              <li key={e.id}>
                {e.description} <span className="tabular-nums text-neutral">× {e.quantity}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {diff.changed.length > 0 && (
        <div>
          <h4 className="font-medium text-warning">Quantity changed ({diff.changed.length})</h4>
          <ul>
            {diff.changed.map((c) => (
              <li key={c.id}>
                {c.description} <span className="tabular-nums text-neutral">{c.from} → {c.to}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

function VersionRow({
  version,
  onChanged,
}: {
  version: VersionSummary
  onChanged: () => void
}): ReactElement {
  const [diff, setDiff] = useState<BookDiff | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const open = async (): Promise<void> => {
    setBusy(true)
    setError(null)
    try {
      // Read the book and the version fresh, so the diff is against what is live now.
      const [positions, specs] = await Promise.all([
        api.positions(),
        api.portfolioVersionSpecs(version.file),
      ])
      setDiff(diffBooks(positions.map(entryFromRisk), specs.map(entryFromSpec)))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const confirm = async (): Promise<void> => {
    setBusy(true)
    setError(null)
    try {
      await api.loadPortfolioVersion(version.file)
      setDiff(null)
      onChanged()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <li className="flex flex-col gap-2 py-2" data-testid={`version-${version.file}`}>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
        <span className="font-medium">{version.name}</span>
        <span
          className={`rounded px-1.5 py-0.5 text-xs ${version.auto ? 'bg-surface text-neutral' : 'bg-info/20 text-info'}`}
        >
          {version.auto ? 'auto' : 'named'}
        </span>
        <span className="tabular-nums text-neutral">
          {formatSimTime(version.saved_at, { seconds: false, withDate: true })}
        </span>
        <span className="tabular-nums text-neutral">{formatBytes(version.size_bytes)}</span>
        <button
          type="button"
          disabled={busy || diff !== null}
          onClick={() => void open()}
          className="ml-auto rounded border border-border px-2 py-1 text-xs hover:bg-surface disabled:opacity-50"
        >
          Load…
        </button>
      </div>
      {diff !== null && (
        <div className="flex flex-col gap-2 rounded border border-border bg-surface p-3" data-testid="load-confirm">
          <p className="text-sm">Loading this version replaces the live book. It would change:</p>
          <DiffSummary diff={diff} />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void confirm()}
              className="rounded bg-info px-3 py-1 text-sm text-surface disabled:opacity-50"
            >
              Load version
            </button>
            <button
              type="button"
              onClick={() => setDiff(null)}
              className="rounded border border-border px-3 py-1 text-sm"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && <p role="alert" className="text-xs text-down">{error}</p>}
    </li>
  )
}

/** The Portfolios page: the current book, saved versions, saving a named version, and loading
 * one after seeing what it would change. Uses REST only, so it needs no stream. */
export function PortfoliosPage(): ReactElement {
  const book = useLoad(() => api.positions(), [])
  const versions = useLoad(() => api.portfolioVersions(), [])
  const [name, setName] = useState('')
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)

  const save = async (): Promise<void> => {
    setSaving(true)
    setSaveError(null)
    try {
      await api.savePortfolioVersion({ name: name.trim() })
      setName('')
      versions.reload()
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  const newest = [...(versions.data ?? [])].sort((a, b) => b.saved_at.localeCompare(a.saved_at))

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-base font-semibold">Portfolios</h2>
      <section aria-label="Current book" className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">Current book</h3>
          <button type="button" onClick={book.reload} className="rounded border border-border px-2 py-1 text-xs hover:bg-surface">
            Refresh
          </button>
        </div>
        {book.error && <p role="alert" className="text-sm text-down">{book.error}</p>}
        {book.data && <PositionsTable positions={book.data} readOnly />}
      </section>

      <section aria-label="Saved versions" className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">Saved versions</h3>
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            if (name.trim() !== '') void save()
          }}
        >
          <input
            aria-label="Version name"
            placeholder="Version name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="rounded border border-border bg-surface px-2 py-1 text-sm"
          />
          <button
            type="submit"
            disabled={saving || name.trim() === ''}
            className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
          >
            Save as version
          </button>
          {saveError && <span role="alert" className="text-xs text-down">{saveError}</span>}
        </form>
        {versions.error && <p role="alert" className="text-sm text-down">{versions.error}</p>}
        {versions.data?.length === 0 && <p className="text-sm text-neutral">No saved versions yet.</p>}
        <ul className="flex flex-col divide-y divide-border">
          {newest.map((version) => (
            <VersionRow
              key={version.file}
              version={version}
              onChanged={() => {
                // The engine applies a load on its next tick, so give it a moment.
                setTimeout(book.reload, 500)
              }}
            />
          ))}
        </ul>
      </section>
    </div>
  )
}
