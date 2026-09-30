import type { ReactElement } from 'react'

import { useThrottledHistory } from '../hooks/useThrottledHistory'
import {
  useConnectionStatus,
  usePolledPositions,
  usePolledSummary,
  useSnapshot,
} from '../store/riskStore'
import { AlertsPane } from './AlertsPane'
import { AllocationDonut } from './AllocationDonut'
import { Controls } from './Controls'
import { GreeksCharts } from './GreeksCharts'
import { PnlAttributionPanel } from './PnlAttributionPanel'
import { PositionsTable } from './PositionsTable'
import { RiskTiles } from './RiskTiles'

/**
 * The thin container Part 15's prompt asks for: it is the only place in the
 * dashboard that reads the store, and it hands every panel plain props. Part
 * 21 feeds the same panels from a backtest replay stream instead of this
 * store, which only this file needs to know about.
 *
 * While the socket is open, live figures come from the latest `RiskSnapshot`.
 * While it is down, positions/totals/limits/compute_micros fall back to the
 * polling client's last response (see `src/api/stream.ts`); alerts, today's P&L,
 * the VaR tile's `VarReport` (Part 18) and the Part 16 charts below (allocation's
 * `by_underlying`, the whole `PnlReport`, and `history`, none of which any polling
 * route carries) have no polling fallback, so they simply hold their last known
 * snapshot value, same as the header (see `Header.tsx`).
 */
export function Dashboard(): ReactElement {
  const status = useConnectionStatus()
  const snapshot = useSnapshot()
  const polledSummary = usePolledSummary()
  const polledPositions = usePolledPositions()
  const history = useThrottledHistory()

  const live = status === 'open'
  const positions = live ? (snapshot?.positions ?? []) : (polledPositions ?? snapshot?.positions ?? [])
  const totals = live
    ? (snapshot?.summary.totals ?? null)
    : (polledSummary?.totals ?? snapshot?.summary.totals ?? null)
  const limits = live ? (snapshot?.limits ?? []) : (polledSummary?.limits ?? snapshot?.limits ?? [])
  const computeMicros = live
    ? (snapshot?.compute_micros ?? null)
    : (polledSummary?.compute_micros ?? snapshot?.compute_micros ?? null)
  const concentration = live
    ? (snapshot?.summary.concentration ?? [])
    : (polledSummary?.concentration ?? snapshot?.summary.concentration ?? [])

  return (
    <>
      <PositionsTable positions={positions} className="lg:col-span-2" />
      <RiskTiles
        totals={totals}
        limits={limits}
        pnlToday={snapshot?.pnl.portfolio.today.actual ?? null}
        computeMicros={computeMicros}
        varReport={snapshot?.var ?? null}
      />
      <AlertsPane
        active={snapshot?.active_alerts ?? []}
        history={snapshot?.alert_history ?? []}
        className="lg:col-span-2"
      />
      <Controls paused={snapshot?.paused ?? null} clockSpeed={snapshot?.clock_speed ?? null} />
      <GreeksCharts history={history} limits={limits} className="lg:col-span-3" />
      <AllocationDonut concentration={concentration} byUnderlying={snapshot?.summary.by_underlying ?? {}} />
      <PnlAttributionPanel pnl={snapshot?.pnl ?? null} className="lg:col-span-2" />
    </>
  )
}
