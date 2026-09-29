import type { ReactElement } from 'react'

import {
  useConnectionStatus,
  usePolledPositions,
  usePolledSummary,
  useSnapshot,
} from '../store/riskStore'
import { AlertsPane } from './AlertsPane'
import { Controls } from './Controls'
import { PlaceholderPanel } from './PlaceholderPanel'
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
 * polling client's last response (see `src/api/stream.ts`); alerts and
 * today's P&L have no polling route, so they simply hold their last known
 * snapshot value, same as the header (see `Header.tsx`).
 */
export function Dashboard(): ReactElement {
  const status = useConnectionStatus()
  const snapshot = useSnapshot()
  const polledSummary = usePolledSummary()
  const polledPositions = usePolledPositions()

  const live = status === 'open'
  const positions = live ? (snapshot?.positions ?? []) : (polledPositions ?? snapshot?.positions ?? [])
  const totals = live
    ? (snapshot?.summary.totals ?? null)
    : (polledSummary?.totals ?? snapshot?.summary.totals ?? null)
  const limits = live ? (snapshot?.limits ?? []) : (polledSummary?.limits ?? snapshot?.limits ?? [])
  const computeMicros = live
    ? (snapshot?.compute_micros ?? null)
    : (polledSummary?.compute_micros ?? snapshot?.compute_micros ?? null)

  return (
    <>
      <PositionsTable positions={positions} className="lg:col-span-2" />
      <RiskTiles
        totals={totals}
        limits={limits}
        pnlToday={snapshot?.pnl.portfolio.today.actual ?? null}
        computeMicros={computeMicros}
      />
      <AlertsPane
        active={snapshot?.active_alerts ?? []}
        history={snapshot?.alert_history ?? []}
        className="lg:col-span-2"
      />
      <Controls paused={snapshot?.paused ?? null} clockSpeed={snapshot?.clock_speed ?? null} />
      <PlaceholderPanel title="Charts" note="Greeks, allocation and P&L charts land in Part 16." />
    </>
  )
}
