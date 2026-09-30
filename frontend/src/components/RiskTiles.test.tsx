import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { LimitStatus } from '../api/generated/LimitStatus'
import type { PortfolioTotals } from '../api/generated/PortfolioTotals'
import type { VarMethodReport } from '../api/generated/VarMethodReport'
import type { VarReport } from '../api/generated/VarReport'
import { RiskTiles } from './RiskTiles'

function totals(overrides: Partial<PortfolioTotals> = {}): PortfolioTotals {
  return {
    dollar_delta: 100_000,
    dollar_gamma_1pct: 500,
    vega: 200,
    theta: -300,
    rho: 0,
    market_value: 10_000,
    unrealised_pnl: 1_000,
    realised_pnl: 0,
    gross_exposure: 150_000,
    contract_notional: 200_000,
    ...overrides,
  }
}

function limit(overrides: Partial<LimitStatus> = {}): LimitStatus {
  return {
    name: 'portfolio_dollar_delta',
    scope: { kind: 'portfolio' },
    value: 100_000,
    limit: 550_000,
    utilisation: 0.18,
    state: 'ok',
    ...overrides,
  }
}

function varMethodReport(overrides: Partial<VarMethodReport> = {}): VarMethodReport {
  return {
    var_95: 10_000,
    var_99: 15_000,
    es_97_5: 18_000,
    scenario_count: 10_000,
    window_start: null,
    window_end: null,
    compute_micros: 200,
    ...overrides,
  }
}

function varReport(overrides: Partial<VarReport> = {}): VarReport {
  return {
    headline_method: 'delta_gamma_mc',
    delta_normal: varMethodReport({ var_99: 5_000 }),
    delta_gamma_mc: varMethodReport({ var_99: 15_000 }),
    historical: varMethodReport({ var_99: 20_000 }),
    total_compute_micros: 600,
    freshly_computed: true,
    recompute_every_ticks: 1,
    ...overrides,
  }
}

describe('RiskTiles', () => {
  it('shows a waiting message with no totals yet', () => {
    render(<RiskTiles totals={null} limits={[]} pnlToday={null} computeMicros={null} varReport={null} />)
    expect(screen.getByText(/waiting for data/i)).toBeInTheDocument()
  })

  it.each([
    ['ok', 'bg-up'],
    ['warning', 'bg-warning'],
    ['breach', 'bg-breach'],
  ] as const)('colours the %s utilisation bar %s', (state, expectedClass) => {
    render(
      <RiskTiles
        totals={totals()}
        limits={[limit({ state, utilisation: state === 'ok' ? 0.2 : state === 'warning' ? 0.85 : 1.1 })]}
        pnlToday={500}
        computeMicros={1200}
        varReport={null}
      />,
    )
    const tile = screen.getByTestId('tile-dollar_delta')
    expect(tile).toHaveAttribute('data-state', state)
    const bar = tile.querySelector(`.${expectedClass}`)
    expect(bar).not.toBeNull()
  })

  it('shows a tile with no limit bar when the limit is not configured', () => {
    render(<RiskTiles totals={totals()} limits={[]} pnlToday={0} computeMicros={100} varReport={null} />)
    const tile = screen.getByTestId('tile-dollar_delta')
    expect(tile.querySelector('.bg-up, .bg-warning, .bg-breach')).toBeNull()
  })

  it("renders today's P&L and compute_micros tiles, which are never limited", () => {
    render(<RiskTiles totals={totals()} limits={[]} pnlToday={-250} computeMicros={850} varReport={null} />)
    expect(screen.getByTestId('tile-pnl-today')).toHaveTextContent('$250')
    expect(screen.getByTestId('tile-compute-micros')).toHaveTextContent('850 µs')
  })

  it('shows a placeholder in the VaR tile before the first VarReport arrives', () => {
    render(<RiskTiles totals={totals()} limits={[]} pnlToday={0} computeMicros={100} varReport={null} />)
    const tile = screen.getByTestId('tile-var')
    expect(tile).toHaveTextContent('—')
  })

  it("shows the headline method's VaR99 and label", () => {
    render(
      <RiskTiles totals={totals()} limits={[]} pnlToday={0} computeMicros={100} varReport={varReport()} />,
    )
    const tile = screen.getByTestId('tile-var')
    expect(tile).toHaveTextContent('$15.0k')
    expect(tile).toHaveTextContent('Delta-gamma Monte Carlo')
  })

  it('shows all three methods in the hover table, with the headline row bold', () => {
    render(
      <RiskTiles totals={totals()} limits={[]} pnlToday={0} computeMicros={100} varReport={varReport()} />,
    )
    const table = screen.getByTestId('var-hover-table')
    expect(table).toHaveTextContent('Delta-normal')
    expect(table).toHaveTextContent('Delta-gamma Monte Carlo')
    expect(table).toHaveTextContent('Historical simulation')
    const headlineRow = screen.getByText('Delta-gamma Monte Carlo').closest('tr')
    expect(headlineRow).toHaveClass('font-semibold')
  })

  it('colours the VaR tile by the portfolio_var99 limit state', () => {
    render(
      <RiskTiles
        totals={totals()}
        limits={[limit({ name: 'portfolio_var99', state: 'breach', utilisation: 1.2, limit: 12_500 })]}
        pnlToday={0}
        computeMicros={100}
        varReport={varReport()}
      />,
    )
    const tile = screen.getByTestId('tile-var')
    expect(tile).toHaveAttribute('data-state', 'breach')
    expect(tile.querySelector('.bg-breach')).not.toBeNull()
  })
})
