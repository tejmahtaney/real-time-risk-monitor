import { useCallback, useState, type ReactElement, type ReactNode } from 'react'

import { api } from '../api/rest'
import { formatMicros, formatSimTime } from '../lib/format'
import { useConnectionStatus, useSnapshot } from '../store/riskStore'
import { ConnectionDot } from './ConnectionDot'

function formatSpeed(speed: number): string {
  return `${speed % 1 === 0 ? speed.toFixed(0) : speed.toFixed(1)}×`
}

/** The app's header: name, connection dot, sim time and session state, clock speed,
 * a pause/resume control and `compute_micros`. Live figures come from the latest
 * `RiskSnapshot`; while the socket is down these simply hold their last known values,
 * since the polling fallback does not carry a full snapshot (see `src/api/stream.ts`).
 * That is the correct value to show, but not to show unlabelled: while disconnected
 * (`polling` or `closed`), these frozen fields are greyed out with a "stale" marker,
 * comparing `status` itself since a snapshot carries no staleness flag of its own.
 *
 * The header always shows the *live* feed, on the backtest page too: a replay's progress and
 * date are shown by the replay view, from its own `BacktestStatus`. `nav` is the mode switch. */
export function Header({ nav }: { nav?: ReactNode } = {}): ReactElement {
  const status = useConnectionStatus()
  const snapshot = useSnapshot()
  const [pending, setPending] = useState(false)
  const stale = (status === 'polling' || status === 'closed') && snapshot !== null

  const togglePause = useCallback(() => {
    if (!snapshot || pending) return
    setPending(true)
    const run = async (): Promise<void> => {
      try {
        await (snapshot.paused ? api.resume() : api.pause())
      } finally {
        setPending(false)
      }
    }
    void run()
  }, [snapshot, pending])

  return (
    <header className="flex flex-wrap items-center justify-between gap-4 border-b border-border bg-surface-raised px-4 py-3">
      <div className="flex items-center gap-4">
        <h1 className="text-lg font-semibold">Real-Time Risk Monitor</h1>
        <ConnectionDot status={status} />
        {nav}
      </div>
      <div
        className={`flex flex-wrap items-center gap-4 text-sm tabular-nums ${stale ? 'text-neutral/70' : ''}`}
      >
        <span data-testid="sim-time">
          <span className="text-neutral">Sim time </span>
          {snapshot ? formatSimTime(snapshot.sim_time, { withDate: true }) : '—'}
        </span>
        <span data-testid="session-state">
          <span className="text-neutral">Session </span>
          {snapshot ? (snapshot.paused ? 'Paused' : 'Running') : '—'}
          {snapshot?.feed_status === 'degraded' && (
            <span className="ml-1 text-warning">(feed degraded)</span>
          )}
        </span>
        <span data-testid="clock-speed">
          <span className="text-neutral">Speed </span>
          {snapshot ? formatSpeed(snapshot.clock_speed) : '—'}
        </span>
        <span data-testid="compute-micros">
          <span className="text-neutral">Compute </span>
          {snapshot ? formatMicros(snapshot.compute_micros) : '—'}
        </span>
        {stale && (
          <span
            data-testid="stale-marker"
            className="rounded bg-warning/20 px-1.5 py-0.5 text-xs font-medium uppercase tracking-wide text-warning"
            title="Disconnected: these figures are the last ones received, not live."
          >
            Stale
          </span>
        )}
        <button
          type="button"
          onClick={togglePause}
          disabled={!snapshot || pending}
          className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
        >
          {snapshot?.paused ? 'Resume' : 'Pause'}
        </button>
      </div>
    </header>
  )
}
