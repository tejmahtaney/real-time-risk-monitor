import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { LimitStatus } from '../api/generated/LimitStatus'
import type { PortfolioTotals } from '../api/generated/PortfolioTotals'
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

describe('RiskTiles', () => {
  it('shows a waiting message with no totals yet', () => {
    render(<RiskTiles totals={null} limits={[]} pnlToday={null} computeMicros={null} />)
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
      />,
    )
    const tile = screen.getByTestId('tile-dollar_delta')
    expect(tile).toHaveAttribute('data-state', state)
    const bar = tile.querySelector(`.${expectedClass}`)
    expect(bar).not.toBeNull()
  })

  it('shows a tile with no limit bar when the limit is not configured', () => {
    render(<RiskTiles totals={totals()} limits={[]} pnlToday={0} computeMicros={100} />)
    const tile = screen.getByTestId('tile-dollar_delta')
    expect(tile.querySelector('.bg-up, .bg-warning, .bg-breach')).toBeNull()
  })

  it("renders today's P&L and compute_micros tiles, which are never limited", () => {
    render(<RiskTiles totals={totals()} limits={[]} pnlToday={-250} computeMicros={850} />)
    expect(screen.getByTestId('tile-pnl-today')).toHaveTextContent('$250')
    expect(screen.getByTestId('tile-compute-micros')).toHaveTextContent('850 µs')
  })
})
