import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { position } from '../test/fixtures'
import { PositionsTable } from './PositionsTable'

afterEach(() => {
  window.localStorage.clear()
})

describe('PositionsTable adjusted contracts', () => {
  it('marks an adjusted contract with an adj badge and no other row', () => {
    render(
      <PositionsTable
        positions={[
          position({ id: 'a', description: 'AAPL 2026-10-16 C 57.5', adjusted: true }),
          position({ id: 'b', description: 'AAPL stock', kind: 'equity' }),
        ]}
      />,
    )
    expect(screen.getAllByTestId('adj-badge')).toHaveLength(1)
    expect(screen.getByTestId('adj-badge')).toHaveTextContent('adj')
  })
})

describe('PositionsTable grouping and subtotals', () => {
  it('groups rows under their underlying and shows a subtotal row', () => {
    render(
      <PositionsTable
        positions={[
          position({ id: 'a', underlying: 'AAPL', description: 'AAPL stock' }),
          position({ id: 'b', underlying: 'MSFT', description: 'MSFT stock' }),
        ]}
      />,
    )
    expect(screen.getByText('AAPL')).toBeInTheDocument()
    expect(screen.getByText('MSFT')).toBeInTheDocument()
    expect(screen.getAllByText('Subtotal')).toHaveLength(2)
  })

  it('never shows a portfolio total of share delta', () => {
    render(
      <PositionsTable
        positions={[
          position({ id: 'a', underlying: 'AAPL' }),
          position({ id: 'b', underlying: 'MSFT' }),
        ]}
      />,
    )
    expect(screen.queryByText(/portfolio.*share delta/i)).not.toBeInTheDocument()
    expect(screen.queryByTestId('portfolio-share-delta-total')).not.toBeInTheDocument()
  })

  it('shows a placeholder message with no positions', () => {
    render(<PositionsTable positions={[]} />)
    expect(screen.getByText(/no positions/i)).toBeInTheDocument()
  })
})

describe('PositionsTable sorting', () => {
  it('sorts a group by clicking a column header, and toggles direction on a second click', async () => {
    const user = userEvent.setup()
    render(
      <PositionsTable
        positions={[
          position({ id: 'a', underlying: 'AAPL', description: 'AAPL 10', quantity: 10 }),
          position({ id: 'b', underlying: 'AAPL', description: 'AAPL 1', quantity: 1 }),
          position({ id: 'c', underlying: 'AAPL', description: 'AAPL 5', quantity: 5 }),
        ]}
      />,
    )
    const header = screen.getByRole('columnheader', { name: /qty/i })
    await user.click(header)
    let rows = screen.getAllByTestId('position-row')
    expect(rows[0]?.textContent).toContain('AAPL 1')
    expect(rows[2]?.textContent).toContain('AAPL 10')

    await user.click(header)
    rows = screen.getAllByTestId('position-row')
    expect(rows[0]?.textContent).toContain('AAPL 10')
    expect(rows[2]?.textContent).toContain('AAPL 1')
  })

  it('remembers the sort in localStorage inside a try/catch, surviving a re-render', async () => {
    const user = userEvent.setup()
    const { unmount } = render(
      <PositionsTable
        positions={[
          position({ id: 'a', underlying: 'AAPL', description: 'AAPL 10', quantity: 10 }),
          position({ id: 'b', underlying: 'AAPL', description: 'AAPL 1', quantity: 1 }),
        ]}
      />,
    )
    await user.click(screen.getByRole('columnheader', { name: /qty/i }))
    unmount()

    render(
      <PositionsTable
        positions={[
          position({ id: 'a', underlying: 'AAPL', description: 'AAPL 10', quantity: 10 }),
          position({ id: 'b', underlying: 'AAPL', description: 'AAPL 1', quantity: 1 }),
        ]}
      />,
    )
    const rows = screen.getAllByTestId('position-row')
    expect(rows[0]?.textContent).toContain('AAPL 1')
  })

  it('does not throw when localStorage.getItem throws', () => {
    const spy = vi.spyOn(window.localStorage.__proto__, 'getItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    expect(() => render(<PositionsTable positions={[position()]} />)).not.toThrow()
    spy.mockRestore()
  })
})

describe('PositionsTable value-change pulse', () => {
  it('does not pulse a cell whose displayed text is unchanged, even if the raw value moved slightly', () => {
    const { rerender } = render(<PositionsTable positions={[position({ id: 'a', spot: 230 })]} />)
    rerender(<PositionsTable positions={[position({ id: 'a', spot: 230.4 })]} />)
    const cells = screen.getByTestId('position-row').querySelectorAll('td')
    expect(cells[2]?.className).not.toMatch(/text-pulse/)
  })

  it('pulses a cell green with a text-colour class, never a background class, when its displayed value rises', () => {
    const { rerender } = render(<PositionsTable positions={[position({ id: 'a', spot: 230 })]} />)
    rerender(<PositionsTable positions={[position({ id: 'a', spot: 235 })]} />)
    const cells = screen.getByTestId('position-row').querySelectorAll('td')
    expect(cells[2]?.className).toContain('text-pulse-up')
    expect(cells[2]?.className).not.toMatch(/bg-(up|down|breach|warning)/)
  })

  it('pulses red when its displayed value falls', () => {
    const { rerender } = render(<PositionsTable positions={[position({ id: 'a', spot: 235 })]} />)
    rerender(<PositionsTable positions={[position({ id: 'a', spot: 230 })]} />)
    const cells = screen.getByTestId('position-row').querySelectorAll('td')
    expect(cells[2]?.className).toContain('text-pulse-down')
  })
})

describe('PositionsTable status display', () => {
  it('greys an expired row', () => {
    render(<PositionsTable positions={[position({ status: { state: 'expired' } })]} />)
    const row = screen.getByTestId('position-row')
    expect(row.className).toContain('text-neutral/60')
  })

  it("shows an error row's reason in a tooltip", () => {
    render(
      <PositionsTable
        positions={[position({ status: { state: 'error', reason: 'missing underlying' } })]}
      />,
    )
    expect(screen.getByTestId('position-row')).toHaveAttribute('title', 'missing underlying')
  })
})

describe('PositionsTable narrow screens', () => {
  it('scrolls sideways instead of clipping columns', () => {
    render(<PositionsTable positions={[position({ id: 'a' })]} />)
    const table = screen.getByRole('table')
    expect(table).toHaveClass('min-w-max')
    expect(table.parentElement).toHaveClass('overflow-x-auto')
    expect(screen.getAllByRole('columnheader').length).toBeGreaterThanOrEqual(12)
  })
})
