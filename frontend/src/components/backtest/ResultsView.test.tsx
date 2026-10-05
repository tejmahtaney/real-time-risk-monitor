import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { BacktestResults } from '../../api/generated/BacktestResults'
import type { DailyRow } from '../../api/generated/DailyRow'
import { ResultsView } from './ResultsView'

function row(date: string, actual: number, cumulative: number, exception = false): DailyRow {
  return {
    date,
    delta: actual,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
    higher_order: 0,
    dividends: 0,
    residual: 0,
    actual,
    cumulative_pnl: cumulative,
    var_99: 500,
    exception,
    breaches: 0,
  }
}

const ZERO = {
  delta: 0,
  gamma: 0,
  vega: 0,
  theta: 0,
  rho: 0,
  higher_order: 0,
  dividends: 0,
  residual: 0,
  actual: 0,
}

function results(partial: boolean, state: 'finished' | 'unfinished'): BacktestResults {
  return {
    status: { id: 'run-1', state, in_memory: false, steps_done: 2, steps_total: 80 },
    settings: { run_id: 'run-1' },
    rows: [row('2026-06-04', 100, 100), row('2026-06-05', -900, -800, true)],
    summary: {
      days: 2,
      total: { ...ZERO, actual: -800 },
      max_drawdown: { amount: 900, peak_date: '2026-06-04', trough_date: '2026-06-05' },
      best_day: { date: '2026-06-04', pnl: 100 },
      worst_day: { date: '2026-06-05', pnl: -900 },
      days_with_breaches: 0,
      var_backtest: {
        n: 2,
        exceptions: 1,
        expected_exceptions: 0.02,
        lr: 9,
        p_value: 0.004,
        reject_5pct: true,
        low_power: true,
      },
    },
    partial,
    events: [],
  } as unknown as BacktestResults
}

describe('ResultsView', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
  })

  it('shows the summary, the plain-words VaR sentence and highlights exception days', () => {
    render(<ResultsView results={results(false, 'finished')} />)
    expect(screen.getByTestId('var-backtest-sentence')).toHaveTextContent(
      '1 exception against 0.0 expected. p = 0.004, so the model understates risk.',
    )
    expect(screen.getByText('−$800')).toBeInTheDocument()
    const rows = screen.getAllByRole('row')
    const exceptionRow = rows.find((r) => r.getAttribute('data-exception') === 'true')
    expect(exceptionRow).toHaveTextContent('2026-06-05')
    expect(exceptionRow).toHaveClass('bg-breach/20')
    expect(rows.find((r) => r.getAttribute('data-exception') === 'false')).not.toHaveClass(
      'bg-breach/20',
    )
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('marks an unfinished run as covering only the dates it replayed', () => {
    render(<ResultsView results={results(true, 'unfinished')} />)
    expect(screen.getByRole('status')).toHaveTextContent(
      'unfinished: it stopped after 2 of 80 dates',
    )
  })

  it('downloads the daily table as CSV', async () => {
    const createObjectURL = vi.fn(() => 'blob:x')
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL: vi.fn() })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    render(<ResultsView results={results(false, 'finished')} />)
    await userEvent.click(screen.getByRole('button', { name: /Download daily table/ }))
    expect(createObjectURL).toHaveBeenCalledTimes(1)
    const blob = (createObjectURL.mock.calls[0] as unknown as [Blob])[0]
    const text = await new Promise<string>((resolve) => {
      const reader = new FileReader()
      reader.onload = () => resolve(String(reader.result))
      reader.readAsText(blob)
    })
    expect(click).toHaveBeenCalledTimes(1)
    expect(text).toContain('2026-06-05,-900,0,0,0,0,0,0,0,-900,-800,500,true,0')
  })
})
