import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react'

import type { BacktestControlRequest } from '../../api/generated/BacktestControlRequest'
import type { BacktestResults } from '../../api/generated/BacktestResults'
import type { BacktestStatus } from '../../api/generated/BacktestStatus'
import { api } from '../../api/rest'
import { createStreamClient } from '../../api/stream'
import { leaseRun } from '../../lib/runLease'
import {
  useBacktestStore,
  useConnectionStatus,
  useSnapshot,
  useAlerts,
} from '../../store/riskStore'
import { AlertsPane } from '../AlertsPane'
import { PnlAttributionPanel } from '../PnlAttributionPanel'
import { PositionsTable } from '../PositionsTable'
import { RiskTiles } from '../RiskTiles'
import { ResultsView } from './ResultsView'

const STATUS_POLL_MS = 500
const SPEED_DEBOUNCE_MS = 250

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/** The replay panels: the same risk tiles, positions table, P&L chart and alerts pane as the
 * live dashboard, fed from the backtest store. Positions are read-only here, so a replayed
 * book cannot change the live one. */
function ReplayPanels(): ReactElement {
  const snapshot = useSnapshot(useBacktestStore)
  const alerts = useAlerts(useBacktestStore)
  return (
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-3" data-testid="replay-panels">
      <PositionsTable positions={snapshot?.positions ?? []} readOnly className="lg:col-span-2" />
      <RiskTiles
        totals={snapshot?.summary.totals ?? null}
        limits={snapshot?.limits ?? []}
        pnlToday={snapshot?.pnl.portfolio.today.actual ?? null}
        computeMicros={snapshot?.compute_micros ?? null}
        varReport={snapshot?.var ?? null}
      />
      <AlertsPane
        active={snapshot?.active_alerts ?? []}
        history={snapshot?.alert_history ?? alerts}
        corporateActions={snapshot?.corporate_actions ?? []}
        className="lg:col-span-2"
      />
      <PnlAttributionPanel pnl={snapshot?.pnl ?? null} />
    </div>
  )
}

/** A live replay: play, pause, step, run to end, a speed slider and a progress bar, with the
 * dashboard panels below and the results once the run finishes.
 *
 * The run is held in server memory only while this view is mounted (see `runLease.ts`), and its
 * stream is closed when the view goes. Progress and play state come from `BacktestStatus`, never
 * from the live snapshot, because in a replay `paused` is always false. */
export function ReplayView({ id, onClose }: { id: string; onClose: () => void }): ReactElement {
  const [status, setStatus] = useState<BacktestStatus | null>(null)
  const [results, setResults] = useState<BacktestResults | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [speed, setSpeed] = useState<number | null>(null)
  const speedTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const connection = useConnectionStatus(useBacktestStore)
  const finished = status?.state === 'finished'

  useEffect(() => {
    useBacktestStore.getState().reset()
    const release = leaseRun(id)
    const client = createStreamClient({ source: { kind: 'backtest', id } })
    client.start()
    return () => {
      client.stop()
      release()
    }
  }, [id])

  useEffect(() => {
    let cancelled = false
    const poll = (): void => {
      api
        .backtestStatus(id)
        .then((next) => {
          if (!cancelled) setStatus(next)
        })
        .catch((err: unknown) => {
          if (!cancelled) setError(errorText(err))
        })
    }
    poll()
    const timer = setInterval(poll, STATUS_POLL_MS)
    return () => {
      cancelled = true
      clearInterval(timer)
    }
  }, [id])

  useEffect(() => {
    if (!finished) return undefined
    let cancelled = false
    api
      .backtestResults(id)
      .then((next) => {
        if (!cancelled) setResults(next)
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(errorText(err))
      })
    return () => {
      cancelled = true
    }
  }, [id, finished])

  useEffect(
    () => () => {
      if (speedTimer.current) clearTimeout(speedTimer.current)
    },
    [],
  )

  const send = useCallback(
    async (request: BacktestControlRequest): Promise<void> => {
      setError(null)
      try {
        setStatus(await api.backtestControl(id, request))
      } catch (err) {
        setError(errorText(err))
      }
    },
    [id],
  )

  const onSpeed = (value: number): void => {
    setSpeed(value)
    if (speedTimer.current) clearTimeout(speedTimer.current)
    speedTimer.current = setTimeout(() => {
      void send({ action: 'set_speed', speed: value })
    }, SPEED_DEBOUNCE_MS)
  }

  const playing = status?.state === 'playing'
  const busyToEnd = status?.state === 'running_to_end'
  const disabled = status === null || finished || status.state === 'unfinished'
  const progress = status?.progress ?? 0
  const shownSpeed = speed ?? status?.speed ?? 5

  return (
    <div className="flex flex-col gap-4">
      <section
        aria-label="Replay controls"
        className="flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4"
      >
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            disabled={disabled || busyToEnd}
            onClick={() => void send({ action: playing ? 'pause' : 'play' })}
            className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
          >
            {playing ? 'Pause' : 'Play'}
          </button>
          <button
            type="button"
            disabled={disabled || playing || busyToEnd}
            onClick={() => void send({ action: 'step' })}
            className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
          >
            Step
          </button>
          <button
            type="button"
            disabled={disabled || busyToEnd}
            onClick={() => void send({ action: 'run_to_end' })}
            className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
          >
            Run to end
          </button>
          <label className="flex items-center gap-2 text-sm">
            <span className="text-neutral">Speed</span>
            <input
              type="range"
              min={1}
              max={50}
              step={1}
              value={shownSpeed}
              aria-label="Replay speed in dates per second"
              disabled={disabled}
              onChange={(event) => onSpeed(Number(event.target.value))}
            />
            <span className="w-24 tabular-nums">{shownSpeed} dates/s</span>
          </label>
          <span className="ml-auto flex items-center gap-3 text-xs text-neutral">
            <span data-testid="replay-stream">Replay stream: {connection}</span>
            <button
              type="button"
              onClick={onClose}
              className="rounded border border-border px-3 py-1 text-sm text-neutral hover:bg-surface"
            >
              Close run
            </button>
          </span>
        </div>
        <div className="flex flex-col gap-1">
          <div
            role="progressbar"
            aria-label="Replay progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(progress * 100)}
            className="h-2 w-full overflow-hidden rounded bg-surface"
          >
            <div className="h-full bg-info" style={{ width: `${progress * 100}%` }} />
          </div>
          <p className="text-xs tabular-nums text-neutral" data-testid="replay-progress">
            {status
              ? `${status.current_date}: ${status.steps_done} of ${status.steps_total} dates (${status.state.replace('_', ' ')})`
              : 'Starting…'}
          </p>
        </div>
        {error && (
          <p role="alert" className="text-sm text-down">
            {error}
          </p>
        )}
      </section>

      {finished ? (
        results ? (
          <ResultsView results={results} />
        ) : (
          <p className="text-sm text-neutral">Loading the results…</p>
        )
      ) : (
        <ReplayPanels />
      )}
    </div>
  )
}
