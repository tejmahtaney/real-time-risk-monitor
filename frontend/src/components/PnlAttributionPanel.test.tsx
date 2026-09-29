import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'

import type { PnlBuckets } from '../api/generated/PnlBuckets'
import type { PnlReport } from '../api/generated/PnlReport'
import { PnlAttributionPanel } from './PnlAttributionPanel'

function buckets(overrides: Partial<PnlBuckets> = {}): PnlBuckets {
  return { delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, higher_order: 0, residual: 0, actual: 0, ...overrides }
}

function report(overrides: Partial<PnlReport> = {}): PnlReport {
  return {
    session_date: '2026-09-29',
    portfolio: {
      today: buckets({ theta: 2_100, vega: -5_000, residual: 50, actual: -2_850 }),
      since_start: buckets({ theta: 10_000, vega: -8_000 }),
    },
    by_underlying: {
      SPY: { today: buckets({ theta: 2_100 }), since_start: buckets() },
      AAPL: { today: buckets({ vega: -5_000 }), since_start: buckets() },
    },
    by_position: {},
    previous_day: null,
    ...overrides,
  }
}

describe('PnlAttributionPanel', () => {
  it('shows a waiting message with no report yet', () => {
    render(<PnlAttributionPanel pnl={null} />)
    expect(screen.getByTestId('pnl-headline')).toHaveTextContent(/waiting for data/i)
  })

  it("builds the headline from today's buckets", () => {
    render(<PnlAttributionPanel pnl={report()} />)
    expect(screen.getByTestId('pnl-headline')).toHaveTextContent(
      'Today you made $2.1k from theta and lost $5.0k to vega. Net −$2.9k.',
    )
  })

  it('defaults to the "Today" period, pressed', () => {
    render(<PnlAttributionPanel pnl={report()} />)
    expect(screen.getByRole('button', { name: 'Today' })).toHaveAttribute('aria-pressed', 'true')
  })

  it('shows a no-data message for a period with nothing yet (previous day before the first roll)', async () => {
    const user = userEvent.setup()
    render(<PnlAttributionPanel pnl={report()} />)
    await user.click(screen.getByRole('button', { name: 'Previous day' }))
    expect(screen.getByText(/no p&l data for previous day yet/i)).toBeInTheDocument()
    // The headline is always built from today's buckets, regardless of the period toggle.
    expect(screen.getByTestId('pnl-headline')).toHaveTextContent('Net −$2.9k.')
  })

  it('renders a stacked by-underlying breakdown when the book has more than one underlying', () => {
    render(<PnlAttributionPanel pnl={report()} />)
    expect(screen.getByText('By underlying')).toBeInTheDocument()
  })

  it('renders nothing extra for the by-underlying section when there are no underlyings', () => {
    render(<PnlAttributionPanel pnl={report({ by_underlying: {} })} />)
    expect(screen.queryByText('By underlying')).not.toBeInTheDocument()
  })
})
