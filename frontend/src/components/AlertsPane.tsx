import { useEffect, useRef, useState, type ReactElement } from 'react'

import type { AlertEvent } from '../api/generated/AlertEvent'
import type { LimitStatus } from '../api/generated/LimitStatus'
import { formatCompactCurrency, formatSimTime } from '../lib/format'
import { limitLabel, scopeLabel, stateRank } from '../lib/limits'

function sortActive(active: LimitStatus[]): LimitStatus[] {
  return [...active].sort((a, b) => {
    const byState = stateRank(b.state) - stateRank(a.state)
    return byState !== 0 ? byState : b.utilisation - a.utilisation
  })
}

function sortHistory(history: AlertEvent[]): AlertEvent[] {
  return [...history].sort((a, b) => b.id - a.id)
}

/** The alerts pane: active warnings and breaches at the top (severity, then
 * utilisation), history below (newest first), and a short border pulse when a
 * new breach arrives. Reads only its props, never the store directly. */
export function AlertsPane({
  active,
  history,
  className = '',
}: {
  active: LimitStatus[]
  history: AlertEvent[]
  className?: string
}): ReactElement {
  const sortedHistory = sortHistory(history)
  const newestBreachId = sortedHistory.find((e) => e.to === 'breach')?.id
  const lastSeenBreachId = useRef<number | undefined>(undefined)
  const [pulsing, setPulsing] = useState(false)

  useEffect(() => {
    if (newestBreachId !== undefined && newestBreachId !== lastSeenBreachId.current) {
      if (lastSeenBreachId.current !== undefined) {
        setPulsing(true)
        const timer = setTimeout(() => setPulsing(false), 900)
        lastSeenBreachId.current = newestBreachId
        return () => clearTimeout(timer)
      }
      lastSeenBreachId.current = newestBreachId
    }
    return undefined
  }, [newestBreachId])

  return (
    <section
      className={`flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4 ${
        pulsing ? 'pane-pulse' : ''
      } ${className}`}
      aria-label="Alerts"
      data-testid="alerts-pane"
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Alerts</h2>

      <div data-testid="active-alerts" className="flex flex-col gap-1">
        {sortActive(active).length === 0 ? (
          <p className="text-sm text-neutral">No active warnings or breaches.</p>
        ) : (
          sortActive(active).map((status) => (
            <div
              key={`${status.name}-${status.scope.kind === 'portfolio' ? 'portfolio' : status.scope.symbol}`}
              className={`flex items-center justify-between rounded px-2 py-1 text-sm ${
                status.state === 'breach' ? 'bg-breach/20 text-breach' : 'bg-warning/20 text-warning'
              }`}
              data-testid="active-alert"
              data-state={status.state}
            >
              <span>
                {limitLabel(status.name)} · {scopeLabel(status.scope)}
              </span>
              <span className="tabular-nums">
                {formatCompactCurrency(status.value)} / {formatCompactCurrency(status.limit)}
              </span>
            </div>
          ))
        )}
      </div>

      <div className="flex flex-col gap-1">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-neutral">History</h3>
        <div className="max-h-64 overflow-auto text-sm" data-testid="alert-history">
          {sortedHistory.length === 0 ? (
            <p className="text-neutral">No alert history yet.</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {sortedHistory.map((event) => (
                <li key={event.id} className="flex items-center justify-between gap-2 tabular-nums">
                  <span className="text-neutral">{formatSimTime(event.sim_time)}</span>
                  <span>
                    {limitLabel(event.name)} ({scopeLabel(event.scope)})
                  </span>
                  <span className="capitalize">
                    {event.from} → {event.to}
                  </span>
                  <span>
                    {formatCompactCurrency(event.value)} / {formatCompactCurrency(event.limit)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </section>
  )
}
