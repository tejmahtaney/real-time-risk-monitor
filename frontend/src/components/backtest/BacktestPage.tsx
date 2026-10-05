import { useEffect, useState, type ReactElement } from 'react'

import type { BacktestResults } from '../../api/generated/BacktestResults'
import { api } from '../../api/rest'
import { ReplayView } from './ReplayView'
import { ResultsView } from './ResultsView'
import { SetupView } from './SetupView'

type View = { kind: 'setup' } | { kind: 'replay'; id: string } | { kind: 'saved'; id: string }

/** A saved run's results, read straight from its saved files with no stream and no lease. */
function SavedRun({ id, onBack }: { id: string; onBack: () => void }): ReactElement {
  const [results, setResults] = useState<BacktestResults | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .backtestResults(id)
      .then((value) => {
        if (!cancelled) setResults(value)
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err))
      })
    return () => {
      cancelled = true
    }
  }, [id])

  return (
    <div className="flex flex-col gap-4">
      <button
        type="button"
        onClick={onBack}
        className="self-start rounded border border-border px-3 py-1 text-sm hover:bg-surface"
      >
        Back to saved runs
      </button>
      {error && (
        <p role="alert" className="text-sm text-down">
          {error}
        </p>
      )}
      {results ? (
        <ResultsView results={results} />
      ) : (
        !error && <p className="text-sm text-neutral">Loading the results…</p>
      )}
    </div>
  )
}

/** The backtester page: the setup view, a live replay, or a saved run's results. Leaving the
 * page unmounts the replay, which stops its stream and frees the run's memory on the server; the
 * run's files stay. */
export function BacktestPage(): ReactElement {
  const [view, setView] = useState<View>({ kind: 'setup' })
  switch (view.kind) {
    case 'replay':
      return <ReplayView key={view.id} id={view.id} onClose={() => setView({ kind: 'setup' })} />
    case 'saved':
      return <SavedRun key={view.id} id={view.id} onBack={() => setView({ kind: 'setup' })} />
    case 'setup':
      return (
        <SetupView
          onStarted={(id) => setView({ kind: 'replay', id })}
          onOpenSaved={(id) => setView({ kind: 'saved', id })}
        />
      )
  }
}
