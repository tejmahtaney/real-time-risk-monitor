import { beforeEach, describe, expect, it } from 'vitest'

import type { AlertEvent } from '../api/generated/AlertEvent'
import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { RiskSnapshot } from '../api/generated/RiskSnapshot'
import { ALERT_CAP, HISTORY_CAP, historyPointFromSnapshot, useRiskStore } from './riskStore'

function baseSnapshot(overrides: Partial<RiskSnapshot> = {}): RiskSnapshot {
  return {
    seq: 1,
    wall_time: '2026-09-28T14:30:00Z',
    sim_time: '2026-09-28T14:30:00Z',
    session_date: '2026-09-28',
    step_kind: 'intraday',
    market: {},
    positions: [],
    summary: {
      by_underlying: {},
      totals: {
        dollar_delta: 100,
        dollar_gamma_1pct: 0,
        vega: 0,
        theta: 0,
        rho: 0,
        market_value: 0,
        unrealised_pnl: 0,
        realised_pnl: 0,
        gross_exposure: 0,
        contract_notional: 0,
      },
      concentration: [],
      active_positions: 0,
      expired_positions: 0,
      error_positions: 0,
    },
    pnl: {
      session_date: '2026-09-28',
      portfolio: {
        today: {
          delta: 0,
          gamma: 0,
          vega: 0,
          theta: 0,
          rho: 0,
          higher_order: 0,
          residual: 0,
          actual: 0,
        },
        since_start: {
          delta: 0,
          gamma: 0,
          vega: 0,
          theta: 0,
          rho: 0,
          higher_order: 0,
          residual: 0,
          actual: 0,
        },
      },
      by_underlying: {},
      by_position: {},
      previous_day: null,
    },
    active_alerts: [],
    limits: [
      {
        name: 'portfolio_dollar_delta',
        scope: { kind: 'portfolio' },
        value: 500,
        limit: 1000,
        utilisation: 0.5,
        state: 'ok',
      },
      {
        name: 'portfolio_vega',
        scope: { kind: 'portfolio' },
        value: 800,
        limit: 1000,
        utilisation: 0.8,
        state: 'warning',
      },
    ],
    alert_history: [],
    clock_speed: 1,
    paused: false,
    feed_status: 'ok',
    compute_micros: 100,
    ...overrides,
  }
}

function alertEvent(overrides: Partial<AlertEvent> = {}): AlertEvent {
  return {
    id: 1,
    wall_time: '2026-09-28T14:30:00Z',
    sim_time: '2026-09-28T14:30:00Z',
    name: 'portfolio_dollar_delta',
    scope: { kind: 'portfolio' },
    from: 'ok',
    to: 'warning',
    value: 85_000,
    limit: 100_000,
    ...overrides,
  }
}

beforeEach(() => {
  useRiskStore.getState().reset()
})

describe('applySnapshot', () => {
  it('stores the snapshot and derives a history point from it', () => {
    useRiskStore.getState().applySnapshot(baseSnapshot({ seq: 7 }))
    const state = useRiskStore.getState()
    expect(state.snapshot?.seq).toBe(7)
    expect(state.history).toHaveLength(1)
    expect(state.history[0]?.seq).toBe(7)
    expect(state.history[0]?.dollar_delta).toBe(100)
  })

  it('takes the worst limit utilisation across every configured limit', () => {
    useRiskStore.getState().applySnapshot(baseSnapshot())
    expect(useRiskStore.getState().history[0]?.worst_limit_utilisation).toBe(0.8)
  })

  it('deduplicates a re-applied seq rather than growing the history', () => {
    useRiskStore.getState().applySnapshot(baseSnapshot({ seq: 1 }))
    useRiskStore.getState().applySnapshot(baseSnapshot({ seq: 1 }))
    expect(useRiskStore.getState().history).toHaveLength(1)
  })
})

describe('history cap', () => {
  it('keeps at most HISTORY_CAP points, dropping the oldest', () => {
    const points: HistoryPoint[] = Array.from({ length: HISTORY_CAP + 50 }, (_, i) => ({
      ...historyPointFromSnapshot(baseSnapshot()),
      seq: i,
    }))
    useRiskStore.getState().mergeHistory(points)
    const state = useRiskStore.getState()
    expect(state.history).toHaveLength(HISTORY_CAP)
    expect(state.history[0]?.seq).toBe(50)
    expect(state.history[state.history.length - 1]?.seq).toBe(HISTORY_CAP + 49)
  })

  it('keeps history sorted by seq after a merge', () => {
    useRiskStore.getState().mergeHistory([
      { ...historyPointFromSnapshot(baseSnapshot()), seq: 5 },
      { ...historyPointFromSnapshot(baseSnapshot()), seq: 2 },
      { ...historyPointFromSnapshot(baseSnapshot()), seq: 8 },
    ])
    expect(useRiskStore.getState().history.map((p) => p.seq)).toEqual([2, 5, 8])
  })
})

describe('alerts cap', () => {
  it('appends alerts', () => {
    useRiskStore.getState().addAlert(alertEvent({ id: 1 }))
    useRiskStore.getState().addAlert(alertEvent({ id: 2 }))
    expect(useRiskStore.getState().alerts.map((a) => a.id)).toEqual([1, 2])
  })

  it('keeps at most ALERT_CAP events, dropping the oldest', () => {
    for (let i = 0; i < ALERT_CAP + 10; i++) {
      useRiskStore.getState().addAlert(alertEvent({ id: i }))
    }
    const alerts = useRiskStore.getState().alerts
    expect(alerts).toHaveLength(ALERT_CAP)
    expect(alerts[0]?.id).toBe(10)
    expect(alerts[alerts.length - 1]?.id).toBe(ALERT_CAP + 9)
  })
})

describe('status and error', () => {
  it('updates the connection status', () => {
    useRiskStore.getState().setStatus('polling')
    expect(useRiskStore.getState().status).toBe('polling')
  })

  it('records and clears the last error', () => {
    useRiskStore.getState().setError('boom')
    expect(useRiskStore.getState().lastError).toBe('boom')
    useRiskStore.getState().setError(null)
    expect(useRiskStore.getState().lastError).toBeNull()
  })
})
