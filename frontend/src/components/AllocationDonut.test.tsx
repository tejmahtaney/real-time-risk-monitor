import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'

import type { Concentration } from '../api/generated/Concentration'
import type { UnderlyingRisk } from '../api/generated/UnderlyingRisk'
import { AllocationDonut } from './AllocationDonut'

function underlyingRisk(overrides: Partial<UnderlyingRisk> = {}): UnderlyingRisk {
  return {
    spot: 100,
    share_delta: 0,
    dollar_delta: 0,
    share_gamma: 0,
    dollar_gamma_1pct: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    market_value: 0,
    unrealised_pnl: 0,
    realised_pnl: 0,
    gross_exposure: 0,
    contract_notional: 0,
    position_count: 1,
    ...overrides,
  }
}

const CONCENTRATION: Concentration[] = [
  { symbol: 'SPY', gross_exposure: 400_000, fraction: 0.8 },
  { symbol: 'AAPL', gross_exposure: 100_000, fraction: 0.2 },
]

const BY_UNDERLYING: Record<string, UnderlyingRisk> = {
  SPY: underlyingRisk({ vega: 100 }),
  AAPL: underlyingRisk({ vega: -400 }),
}

describe('AllocationDonut', () => {
  it('shows a no-positions message when there is nothing to allocate', () => {
    render(<AllocationDonut concentration={[]} byUnderlying={{}} />)
    expect(screen.getByText(/no positions/i)).toBeInTheDocument()
  })

  it('shows the gross exposure total and legend by default', () => {
    render(<AllocationDonut concentration={CONCENTRATION} byUnderlying={BY_UNDERLYING} />)
    expect(screen.getByTestId('allocation-total')).toHaveTextContent('$500.0k')
    const legend = screen.getByTestId('allocation-legend')
    expect(legend).toHaveTextContent('SPY')
    expect(legend).toHaveTextContent('80.0%')
    expect(legend).toHaveTextContent('AAPL')
    expect(legend).toHaveTextContent('20.0%')
  })

  it('has a toggle between gross exposure and |vega|, gross exposure pressed by default', () => {
    render(<AllocationDonut concentration={CONCENTRATION} byUnderlying={BY_UNDERLYING} />)
    expect(screen.getByRole('button', { name: 'Gross exposure' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: '|Vega|' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('switches the legend to |vega| shares on toggle', async () => {
    const user = userEvent.setup()
    render(<AllocationDonut concentration={CONCENTRATION} byUnderlying={BY_UNDERLYING} />)
    await user.click(screen.getByRole('button', { name: '|Vega|' }))
    expect(screen.getByRole('button', { name: '|Vega|' })).toHaveAttribute('aria-pressed', 'true')
    // |vega|: SPY 100, AAPL 400, total 500 -> AAPL 80%, SPY 20% (inverted from gross exposure).
    const legend = screen.getByTestId('allocation-legend')
    expect(legend).toHaveTextContent('80.0%')
    expect(legend).toHaveTextContent('20.0%')
  })
})
