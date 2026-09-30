import { render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import type { RiskSnapshot } from '../api/generated/RiskSnapshot'
import { useRiskStore } from '../store/riskStore'
import { Header } from './Header'

function snapshot(overrides: Partial<RiskSnapshot> = {}): RiskSnapshot {
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
        dollar_delta: 0,
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
        today: { delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, higher_order: 0, residual: 0, actual: 0 },
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
    limits: [],
    alert_history: [],
    clock_speed: 1,
    paused: false,
    feed_status: 'ok',
    compute_micros: 100,
    var: {
      headline_method: 'delta_gamma_mc',
      delta_normal: {
        var_95: 0,
        var_99: 0,
        es_97_5: 0,
        scenario_count: 0,
        window_start: null,
        window_end: null,
        compute_micros: 0,
      },
      delta_gamma_mc: {
        var_95: 0,
        var_99: 0,
        es_97_5: 0,
        scenario_count: 10_000,
        window_start: null,
        window_end: null,
        compute_micros: 0,
      },
      historical: {
        var_95: 0,
        var_99: 0,
        es_97_5: 0,
        scenario_count: 0,
        window_start: null,
        window_end: null,
        compute_micros: 0,
      },
      total_compute_micros: 0,
      freshly_computed: true,
      recompute_every_ticks: 1,
    },
    ...overrides,
  }
}

afterEach(() => {
  useRiskStore.getState().reset()
})

describe('Header staleness', () => {
  it('shows no stale marker while the socket is open', () => {
    useRiskStore.getState().setStatus('open')
    useRiskStore.getState().applySnapshot(snapshot())
    render(<Header />)
    expect(screen.queryByTestId('stale-marker')).not.toBeInTheDocument()
  })

  it('greys the frozen fields and shows a stale marker while polling', () => {
    useRiskStore.getState().applySnapshot(snapshot())
    useRiskStore.getState().setStatus('polling')
    render(<Header />)
    expect(screen.getByTestId('stale-marker')).toBeInTheDocument()
  })

  it('shows no stale marker before any snapshot has ever arrived', () => {
    useRiskStore.getState().setStatus('polling')
    render(<Header />)
    expect(screen.queryByTestId('stale-marker')).not.toBeInTheDocument()
  })
})
