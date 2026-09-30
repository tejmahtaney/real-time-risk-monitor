import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import type { LimitStatus } from '../api/generated/LimitStatus'
import { GreeksCharts } from './GreeksCharts'

function historyPoint(overrides: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    seq: 1,
    sim_time: new Date('2026-09-29T14:00:00Z').toISOString(),
    dollar_delta: 100_000,
    dollar_gamma_1pct: 500,
    vega: 200,
    theta: -300,
    gross_exposure: 150_000,
    market_value: 10_000,
    pnl_today: { delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, higher_order: 0, residual: 0, actual: 0 },
    worst_limit_utilisation: 0,
    var_99: null,
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

describe('GreeksCharts', () => {
  it('shows a waiting message with no history yet', () => {
    render(<GreeksCharts history={[]} limits={[]} />)
    expect(screen.getByText(/waiting for history/i)).toBeInTheDocument()
  })

  it('renders all five metric charts once there is history, including VaR (Part 18)', () => {
    const history = [
      historyPoint({ seq: 1 }),
      historyPoint({ seq: 2, sim_time: new Date('2026-09-29T14:00:30Z').toISOString() }),
    ]
    render(<GreeksCharts history={history} limits={[limit()]} />)
    expect(screen.getByTestId('greek-chart-Dollar delta ($)')).toBeInTheDocument()
    expect(screen.getByTestId('greek-chart-Dollar gamma 1% ($)')).toBeInTheDocument()
    expect(screen.getByTestId('greek-chart-Vega ($ / vol pt)')).toBeInTheDocument()
    expect(screen.getByTestId('greek-chart-Theta ($ / day)')).toBeInTheDocument()
    expect(screen.getByTestId('greek-chart-VaR 99% ($)')).toBeInTheDocument()
  })

  it('offers a 5m / 15m / all window selector, defaulting to 15m', () => {
    render(<GreeksCharts history={[historyPoint()]} limits={[]} />)
    expect(screen.getByRole('button', { name: '5m' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'All' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '15m' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('switches the pressed window on click', async () => {
    const user = userEvent.setup()
    render(<GreeksCharts history={[historyPoint()]} limits={[]} />)
    await user.click(screen.getByRole('button', { name: 'All' }))
    expect(screen.getByRole('button', { name: 'All' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: '15m' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('shows an offscreen "limit $X" label instead of flattening the chart, when the configured limit is far from the plotted data', () => {
    const history = [historyPoint({ dollar_gamma_1pct: 194 })]
    const gammaLimit = limit({ name: 'portfolio_dollar_gamma1_pct', limit: 9_500, utilisation: 194 / 9_500 })
    render(<GreeksCharts history={history} limits={[gammaLimit]} />)
    const gammaChart = screen.getByTestId('greek-chart-Dollar gamma 1% ($)')
    expect(within(gammaChart).getByTestId('offscreen-limit')).toHaveTextContent('limit $9.5k')
  })

  it('shows no offscreen label when no limit is configured for a metric', () => {
    render(<GreeksCharts history={[historyPoint()]} limits={[]} />)
    const gammaChart = screen.getByTestId('greek-chart-Dollar gamma 1% ($)')
    expect(within(gammaChart).queryByTestId('offscreen-limit')).not.toBeInTheDocument()
  })

  it("the VaR chart's axis follows the data, not a far-away configured limit (Part 18)", () => {
    const history = [
      historyPoint({ seq: 1, var_99: 18_000 }),
      historyPoint({ seq: 2, sim_time: new Date('2026-09-29T14:00:30Z').toISOString(), var_99: 20_000 }),
      historyPoint({ seq: 3, sim_time: new Date('2026-09-29T14:01:00Z').toISOString(), var_99: 22_000 }),
    ]
    const varLimit = limit({ name: 'portfolio_var99', limit: 500_000, utilisation: 22_000 / 500_000 })
    render(<GreeksCharts history={history} limits={[varLimit]} />)
    const varChart = screen.getByTestId('greek-chart-VaR 99% ($)')
    expect(within(varChart).getByTestId('offscreen-limit')).toHaveTextContent('limit $500.0k')
  })

  it('skips history points with a null var_99 (a server built before Part 18, or not yet live)', () => {
    const history = [historyPoint({ seq: 1, var_99: null }), historyPoint({ seq: 2, var_99: 5_000 })]
    render(<GreeksCharts history={history} limits={[]} />)
    expect(screen.getByTestId('greek-chart-VaR 99% ($)')).toBeInTheDocument()
  })
})
