import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { AlertEvent } from '../api/generated/AlertEvent'
import type { CorporateActionEvent } from '../api/generated/CorporateActionEvent'
import type { LimitStatus } from '../api/generated/LimitStatus'
import { AlertsPane } from './AlertsPane'

function active(overrides: Partial<LimitStatus> = {}): LimitStatus {
  return {
    name: 'portfolio_dollar_delta',
    scope: { kind: 'portfolio' },
    value: 90_000,
    limit: 100_000,
    utilisation: 0.9,
    state: 'warning',
    ...overrides,
  }
}

function event(overrides: Partial<AlertEvent> = {}): AlertEvent {
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

describe('AlertsPane ordering', () => {
  it('sorts active alerts by severity, breach before warning', () => {
    render(
      <AlertsPane
        active={[
          active({ name: 'portfolio_vega', state: 'warning', utilisation: 0.95 }),
          active({ name: 'gross_exposure', state: 'breach', utilisation: 1.02 }),
        ]}
        history={[]}
      />,
    )
    const rows = screen.getAllByTestId('active-alert')
    expect(rows[0]).toHaveAttribute('data-state', 'breach')
    expect(rows[1]).toHaveAttribute('data-state', 'warning')
  })

  it('breaks a severity tie by utilisation, highest first', () => {
    render(
      <AlertsPane
        active={[
          active({ name: 'portfolio_vega', state: 'warning', utilisation: 0.81 }),
          active({ name: 'gross_exposure', state: 'warning', utilisation: 0.95 }),
        ]}
        history={[]}
      />,
    )
    const rows = screen.getAllByTestId('active-alert')
    expect(rows[0]).toHaveTextContent('Gross exposure')
    expect(rows[1]).toHaveTextContent('Portfolio vega')
  })

  it('shows history newest first, by id, not insertion order', () => {
    render(
      <AlertsPane
        active={[]}
        history={[
          event({ id: 1, name: 'portfolio_dollar_delta' }),
          event({ id: 3, name: 'gross_exposure' }),
          event({ id: 2, name: 'portfolio_vega' }),
        ]}
      />,
    )
    const items = screen.getByTestId('alert-history').querySelectorAll('li')
    expect(items).toHaveLength(3)
    expect(items[0]?.textContent).toContain('Gross exposure')
    expect(items[1]?.textContent).toContain('Portfolio vega')
    expect(items[2]?.textContent).toContain('Portfolio $ delta')
  })

  it('says so when there are no active alerts', () => {
    render(<AlertsPane active={[]} history={[]} />)
    expect(screen.getByText(/no active warnings or breaches/i)).toBeInTheDocument()
  })
})

function corporateEvent(overrides: Partial<CorporateActionEvent> = {}): CorporateActionEvent {
  return {
    id: 'aapl-4-for-1',
    wall_time: '2026-09-29T13:30:00Z',
    sim_time: '2026-09-29T13:30:00Z',
    action: { type: 'split', symbol: 'AAPL', ratio: 4, effective: '2026-09-29' },
    outcome: 'applied',
    message: 'AAPL 4-for-1 split: 2 positions adjusted',
    positions_adjusted: 2,
    cash_paid: 0,
    ...overrides,
  }
}

describe('AlertsPane corporate actions', () => {
  it('lists them as information, newest first, apart from the limit alerts', () => {
    render(
      <AlertsPane
        active={[]}
        history={[]}
        corporateActions={[
          corporateEvent(),
          corporateEvent({
            id: 'msft-div',
            message: 'MSFT cash dividend of 0.9800 per share: 98.00 cash paid to stock holders',
          }),
        ]}
      />,
    )
    const rows = screen.getAllByTestId('corporate-action')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toHaveTextContent('MSFT cash dividend')
    expect(rows[1]).toHaveTextContent('AAPL 4-for-1 split: 2 positions adjusted')
    // They are not limit alerts.
    expect(screen.getByText('No active warnings or breaches.')).toBeInTheDocument()
    expect(screen.queryAllByTestId('active-alert')).toHaveLength(0)
  })

  it('flags a failed action differently and shows nothing when there are none', () => {
    const { rerender } = render(
      <AlertsPane active={[]} history={[]} corporateActions={[corporateEvent({ outcome: 'failed' })]} />,
    )
    expect(screen.getByTestId('corporate-action')).toHaveAttribute('data-outcome', 'failed')
    rerender(<AlertsPane active={[]} history={[]} />)
    expect(screen.queryByTestId('corporate-actions')).not.toBeInTheDocument()
  })
})
