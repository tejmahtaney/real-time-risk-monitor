import { useCallback, useEffect, useState, type DragEvent, type ReactElement } from 'react'

import type { ArchiveCoverage } from '../../api/generated/ArchiveCoverage'
import type { BacktestListItem } from '../../api/generated/BacktestListItem'
import { api } from '../../api/rest'
import { SavedRunsList } from './SavedRunsList'

const SAMPLE_LONG = `date,symbol,close
2026-01-05,SPY,590.12
2026-01-05,AAPL,243.40
2026-01-06,SPY,592.40
2026-01-06,AAPL,245.05`

const SAMPLE_WIDE = `date,SPY,AAPL
2026-01-05,590.12,243.40
2026-01-06,592.40,245.05`

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/** The page before a run starts: upload a price CSV or replay the archive, and reopen saved
 * runs. Server validation errors (with their line numbers) are shown as they come. */
export function SetupView({
  onStarted,
  onOpenSaved,
}: {
  onStarted: (id: string) => void
  onOpenSaved: (id: string) => void
}): ReactElement {
  const [coverage, setCoverage] = useState<ArchiveCoverage | null>(null)
  const [runs, setRuns] = useState<BacktestListItem[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [dragging, setDragging] = useState(false)

  const loadRuns = useCallback(async (): Promise<void> => {
    try {
      setRuns(await api.backtests())
    } catch (err) {
      setError(errorText(err))
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    api
      .archiveCoverage()
      .then((value) => {
        if (!cancelled) setCoverage(value)
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(errorText(err))
      })
    void loadRuns()
    return () => {
      cancelled = true
    }
  }, [loadRuns])

  const start = async (create: () => Promise<{ id: string }>): Promise<void> => {
    setBusy(true)
    setError(null)
    try {
      const status = await create()
      onStarted(status.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const uploadFile = (file: File | undefined): void => {
    if (file) void start(() => api.uploadBacktest(file))
  }

  const onDrop = (event: DragEvent<HTMLDivElement>): void => {
    event.preventDefault()
    setDragging(false)
    uploadFile(event.dataTransfer.files[0])
  }

  const usable = coverage !== null && coverage.aligned_dates > 0

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <section
          aria-label="Upload prices"
          className="flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4"
        >
          <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">
            Upload prices
          </h2>
          <div
            data-testid="drop-zone"
            onDragOver={(event) => {
              event.preventDefault()
              setDragging(true)
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={onDrop}
            className={`flex flex-col items-center justify-center gap-2 rounded border-2 border-dashed p-6 text-center text-sm ${
              dragging ? 'border-info bg-info/10' : 'border-border'
            }`}
          >
            <p>Drop a daily closing-price CSV here, or</p>
            <label className="cursor-pointer rounded border border-border px-3 py-1 hover:bg-surface">
              Choose a file
              <input
                type="file"
                accept=".csv,text/csv"
                aria-label="Price CSV"
                className="sr-only"
                disabled={busy}
                onChange={(event) => uploadFile(event.target.files?.[0])}
              />
            </label>
            <p className="text-xs text-neutral">
              Up to 5 MB. Symbols the portfolio does not hold are ignored.
            </p>
          </div>
          <div className="flex flex-col gap-1 text-xs text-neutral">
            <p>Long format:</p>
            <pre className="overflow-x-auto rounded bg-surface p-2">{SAMPLE_LONG}</pre>
            <p>Or wide format, one column per symbol:</p>
            <pre className="overflow-x-auto rounded bg-surface p-2">{SAMPLE_WIDE}</pre>
          </div>
        </section>

        <section
          aria-label="Archived history"
          className="flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4"
        >
          <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">
            Archived history
          </h2>
          <p className="text-sm" data-testid="archive-coverage">
            {coverage === null
              ? 'Reading the archive…'
              : usable
                ? `The archive covers ${coverage.aligned_dates} aligned dates, ${coverage.first_date} to ${coverage.last_date}.`
                : coverage.missing_symbols.length > 0
                  ? `No archived history for ${coverage.missing_symbols.join(', ')} yet.`
                  : 'The archive does not hold enough aligned dates yet.'}
          </p>
          <button
            type="button"
            disabled={!usable || busy}
            onClick={() => void start(() => api.createArchiveBacktest())}
            className="self-start rounded bg-info/20 px-3 py-2 text-sm font-medium text-info disabled:opacity-50"
          >
            Use archived real history
          </button>
        </section>
      </div>

      {error && (
        <p
          role="alert"
          className="whitespace-pre-wrap rounded border border-breach bg-breach/10 p-3 text-sm text-down"
        >
          {error}
        </p>
      )}

      <SavedRunsList
        runs={runs}
        onOpen={onOpenSaved}
        onDelete={async (id) => {
          await api.deleteBacktestFiles(id)
          await loadRuns()
        }}
      />
    </div>
  )
}
