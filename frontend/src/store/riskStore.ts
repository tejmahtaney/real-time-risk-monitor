/**
 * The single client-side store for live risk state. Fed by `src/api/stream.ts` (the
 * WebSocket client and its polling fallback); read by the app shell and, from Part 15
 * onwards, the dashboard panels.
 *
 * Selectors are exported individually (see the bottom of this file) rather than
 * handing components the whole store, so a component that only cares about, say, the
 * connection dot does not re-render on every tick's snapshot.
 */
import { create } from 'zustand'

import type { AlertEvent } from '../api/generated/AlertEvent'
import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { PositionRisk } from '../api/generated/PositionRisk'
import type { RiskSnapshot } from '../api/generated/RiskSnapshot'
import type { RiskSummaryResponse } from '../api/generated/RiskSummaryResponse'
import { headlineVar99 } from '../lib/var'

/** How the live data client currently sees the backend. */
export type ConnectionStatus = 'connecting' | 'open' | 'polling' | 'closed'

/** The longest {@link RiskState.history} is allowed to grow. Oldest points are dropped. */
export const HISTORY_CAP = 2_400
/** The longest {@link RiskState.alerts} is allowed to grow. Oldest events are dropped. */
export const ALERT_CAP = 200

interface RiskState {
  status: ConnectionStatus
  snapshot: RiskSnapshot | null
  history: HistoryPoint[]
  alerts: AlertEvent[]
  lastError: string | null
  /** The last risk summary fetched by the polling fallback while the socket is down.
   * `null` until the first poll completes; superseded by live snapshots once the
   * socket reopens. */
  polledSummary: RiskSummaryResponse | null
  /** The last positions list fetched by the polling fallback. See {@link polledSummary}. */
  polledPositions: PositionRisk[] | null

  setStatus: (status: ConnectionStatus) => void
  setError: (error: string | null) => void
  setPolled: (summary: RiskSummaryResponse, positions: PositionRisk[]) => void
  /** Applies a fresh snapshot: replaces the latest snapshot and appends its derived
   * history point (deduplicated and capped). */
  applySnapshot: (snapshot: RiskSnapshot) => void
  /** Merges history points from a backfill or the initial load: deduplicated by `seq`,
   * sorted, and capped at {@link HISTORY_CAP}. */
  mergeHistory: (points: HistoryPoint[]) => void
  addAlert: (alert: AlertEvent) => void
  reset: () => void
}

/** Builds the thin `HistoryPoint` projection of a live snapshot, mirroring exactly how
 * `engine::Engine::tick` builds its own `HistoryPoint` (see `crates/engine/src/engine.rs`
 * and `crates/engine/src/snapshot.rs`), so a client-derived point and a server-backfilled
 * one for the same `seq` are identical. */
export function historyPointFromSnapshot(snapshot: RiskSnapshot): HistoryPoint {
  const worstLimitUtilisation = snapshot.limits.reduce(
    (worst, limit) => Math.max(worst, limit.utilisation),
    0,
  )
  return {
    seq: snapshot.seq,
    sim_time: snapshot.sim_time,
    dollar_delta: snapshot.summary.totals.dollar_delta,
    dollar_gamma_1pct: snapshot.summary.totals.dollar_gamma_1pct,
    vega: snapshot.summary.totals.vega,
    theta: snapshot.summary.totals.theta,
    gross_exposure: snapshot.summary.totals.gross_exposure,
    market_value: snapshot.summary.totals.market_value,
    pnl_today: snapshot.pnl.portfolio.today,
    worst_limit_utilisation: worstLimitUtilisation,
    var_99: headlineVar99(snapshot.var),
  }
}

function mergeHistoryPoints(existing: HistoryPoint[], incoming: HistoryPoint[]): HistoryPoint[] {
  const bySeq = new Map<number, HistoryPoint>()
  for (const point of existing) bySeq.set(point.seq, point)
  for (const point of incoming) bySeq.set(point.seq, point)
  const merged = [...bySeq.values()].sort((a, b) => a.seq - b.seq)
  return merged.length > HISTORY_CAP ? merged.slice(merged.length - HISTORY_CAP) : merged
}

const initialState = {
  status: 'connecting' as ConnectionStatus,
  snapshot: null as RiskSnapshot | null,
  history: [] as HistoryPoint[],
  alerts: [] as AlertEvent[],
  lastError: null as string | null,
  polledSummary: null as RiskSummaryResponse | null,
  polledPositions: null as PositionRisk[] | null,
}

/** Builds one source's slice of client state. The live feed and a backtest replay each get
 * their own, so a replay never overwrites the live dashboard's data. */
export function createRiskStore() {
  return create<RiskState>((set) => ({
    ...initialState,

    setStatus: (status) => set({ status }),
    setError: (lastError) => set({ lastError }),
    setPolled: (polledSummary, polledPositions) => set({ polledSummary, polledPositions }),

    applySnapshot: (snapshot) =>
      set((state) => ({
        snapshot,
        history: mergeHistoryPoints(state.history, [historyPointFromSnapshot(snapshot)]),
      })),

    mergeHistory: (points) =>
      set((state) => ({ history: mergeHistoryPoints(state.history, points) })),

    addAlert: (alert) =>
      set((state) => {
        const alerts = [...state.alerts, alert]
        return {
          alerts: alerts.length > ALERT_CAP ? alerts.slice(alerts.length - ALERT_CAP) : alerts,
        }
      }),

    reset: () => set(initialState),
  }))
}

/** The live feed's store. */
export const useRiskStore = createRiskStore()
/** The backtest replay's store, independent of {@link useRiskStore}. */
export const useBacktestStore = createRiskStore()

/** A store hook, either source's. */
export type RiskStore = typeof useRiskStore

// Selectors: each subscribes a component to exactly one slice, so, for example, the
// alerts pane does not re-render every time a new snapshot arrives.
export const useConnectionStatus = (store: RiskStore = useRiskStore): ConnectionStatus =>
  store((state) => state.status)
export const useSnapshot = (store: RiskStore = useRiskStore): RiskSnapshot | null =>
  store((state) => state.snapshot)
export const useHistory = (store: RiskStore = useRiskStore): HistoryPoint[] =>
  store((state) => state.history)
export const useAlerts = (store: RiskStore = useRiskStore): AlertEvent[] =>
  store((state) => state.alerts)
export const useLastError = (store: RiskStore = useRiskStore): string | null =>
  store((state) => state.lastError)
export const usePolledSummary = (store: RiskStore = useRiskStore): RiskSummaryResponse | null =>
  store((state) => state.polledSummary)
export const usePolledPositions = (store: RiskStore = useRiskStore): PositionRisk[] | null =>
  store((state) => state.polledPositions)
