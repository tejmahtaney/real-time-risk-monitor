import { useState, type ReactElement } from 'react'

import { api } from '../../api/rest'
import { signColor } from '../../lib/format'
import { newestFirst, sessionRowView } from '../../lib/sessions'
import { SessionCompare } from './SessionCompare'
import { SessionDetail } from './SessionDetail'
import { useLoad } from './useLoad'

type View =
  | { kind: 'list' }
  | { kind: 'detail'; id: string }
  | { kind: 'compare'; a: string; b: string }

/** The History page: every saved live session, newest first, to open one or to pick two and
 * compare them. REST only. */
export function HistoryPage(): ReactElement {
  const [view, setView] = useState<View>({ kind: 'list' })
  const [picked, setPicked] = useState<string[]>([])
  const sessions = useLoad(() => api.sessions(), [])

  if (view.kind === 'detail') {
    return <SessionDetail key={view.id} id={view.id} onBack={() => setView({ kind: 'list' })} />
  }
  if (view.kind === 'compare') {
    return <SessionCompare a={view.a} b={view.b} onBack={() => setView({ kind: 'list' })} />
  }

  const toggle = (id: string): void =>
    setPicked((current) =>
      current.includes(id) ? current.filter((x) => x !== id) : current.length < 2 ? [...current, id] : current,
    )
  const [first, second] = picked

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-base font-semibold">History</h2>
        <button
          type="button"
          disabled={first === undefined || second === undefined}
          onClick={() => first !== undefined && second !== undefined && setView({ kind: 'compare', a: first, b: second })}
          className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
        >
          Compare selected ({picked.length}/2)
        </button>
      </div>
      {sessions.error && <p role="alert" className="text-sm text-down">{sessions.error}</p>}
      {sessions.data?.length === 0 && <p className="text-sm text-neutral">No saved sessions yet.</p>}
      {sessions.data && sessions.data.length > 0 && (
        <div className="overflow-x-auto rounded-lg border border-border bg-surface-raised">
          <table className="w-full min-w-max border-collapse text-sm tabular-nums" aria-label="Sessions">
            <thead>
              <tr className="text-left text-neutral">
                <th className="px-2 py-2 font-medium"><span className="sr-only">Compare</span></th>
                <th className="px-2 py-2 font-medium">Start</th>
                <th className="px-2 py-2 font-medium">End</th>
                <th className="px-2 py-2 text-right font-medium">Simulated days</th>
                <th className="px-2 py-2 text-right font-medium">Size</th>
                <th className="px-2 py-2 font-medium">Commit</th>
                <th className="px-2 py-2 text-right font-medium">Total P&amp;L</th>
                <th className="px-2 py-2" />
              </tr>
            </thead>
            <tbody>
              {newestFirst(sessions.data).map((session) => {
                const row = sessionRowView(session)
                return (
                  <tr key={row.id} data-testid={`session-${row.id}`} className="border-t border-border">
                    <td className="px-2 py-1">
                      <input
                        type="checkbox"
                        aria-label={`Compare ${row.id}`}
                        checked={picked.includes(row.id)}
                        disabled={!picked.includes(row.id) && picked.length >= 2}
                        onChange={() => toggle(row.id)}
                      />
                    </td>
                    <td className="whitespace-nowrap px-2 py-1">{row.start}</td>
                    <td className={`whitespace-nowrap px-2 py-1 ${row.cutShort ? 'text-warning' : ''}`}>{row.end}</td>
                    <td className="px-2 py-1 text-right">{row.days}</td>
                    <td className="px-2 py-1 text-right">{row.size}</td>
                    <td className="px-2 py-1 font-mono text-xs">{row.commit}</td>
                    <td className={`px-2 py-1 text-right ${row.pnlValue === null ? 'text-neutral' : signColor(row.pnlValue)}`}>
                      {row.pnl}
                    </td>
                    <td className="px-2 py-1 text-right">
                      <button
                        type="button"
                        onClick={() => setView({ kind: 'detail', id: row.id })}
                        className="rounded border border-border px-2 py-1 text-xs hover:bg-surface"
                      >
                        Open
                      </button>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}
