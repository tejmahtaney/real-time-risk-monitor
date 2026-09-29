import { useCallback, useState, type ReactElement } from 'react'

import { api } from '../api/rest'
import { useConnectionStatus, useSnapshot } from '../store/riskStore'
import { ConnectionDot } from './ConnectionDot'

function formatSimTime(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function formatSpeed(speed: number): string {
  return `${speed % 1 === 0 ? speed.toFixed(0) : speed.toFixed(1)}×`
}

function formatComputeMicros(micros: number): string {
  return micros >= 1_000 ? `${(micros / 1_000).toFixed(2)} ms` : `${micros.toFixed(0)} µs`
}

/** The app's header: name, connection dot, sim time and session state, clock speed,
 * a pause/resume control and `compute_micros`. Live figures come from the latest
 * `RiskSnapshot`; while the socket is down these simply hold their last known values,
 * since the polling fallback does not carry a full snapshot (see `src/api/stream.ts`). */
export function Header(): ReactElement {
  const status = useConnectionStatus()
  const snapshot = useSnapshot()
  const [pending, setPending] = useState(false)

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
      </div>
      <div className="flex flex-wrap items-center gap-4 text-sm tabular-nums">
        <span data-testid="sim-time">
          <span className="text-neutral">Sim time </span>
          {snapshot ? formatSimTime(snapshot.sim_time) : '—'}
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
          {snapshot ? formatComputeMicros(snapshot.compute_micros) : '—'}
        </span>
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
