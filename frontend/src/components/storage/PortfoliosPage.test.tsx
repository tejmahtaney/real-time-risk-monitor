import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { PositionSpec } from '../../api/generated/PositionSpec'
import { api } from '../../api/rest'
import { position } from '../../test/fixtures'
import { PortfoliosPage } from './PortfoliosPage'

afterEach(() => vi.restoreAllMocks())

describe('PortfoliosPage', () => {
  it('shows the diff before loading and loads only on confirmation', async () => {
    vi.spyOn(api, 'positions').mockResolvedValue([
      position({ id: 'keep', description: 'KEEP', quantity: 1 }),
      position({ id: 'gone', description: 'GONE', quantity: 2 }),
    ])
    vi.spyOn(api, 'portfolioVersions').mockResolvedValue([
      { file: '20260928T120227Z-session-start.csv', size_bytes: 10, saved_at: '2026-09-28T12:02:27Z', name: 'session start', auto: true },
    ])
    const spec = (id: string, quantity: number): PositionSpec => ({
      id,
      underlying: 'SPY',
      kind: 'equity',
      strike: null,
      expiry: null,
      quantity,
      entry_price: null,
      iv: null,
    })
    vi.spyOn(api, 'portfolioVersionSpecs').mockResolvedValue([spec('keep', 5), spec('new', 3)])
    const load = vi.spyOn(api, 'loadPortfolioVersion').mockResolvedValue({ position_count: 2 })

    render(<PortfoliosPage />)
    expect(await screen.findByText('auto')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Load…' }))

    const diff = await screen.findByTestId('version-diff')
    expect(diff).toHaveTextContent('Added (1)')
    expect(diff).toHaveTextContent('Removed (1)')
    expect(diff).toHaveTextContent('1 → 5')
    expect(load).not.toHaveBeenCalled()

    await userEvent.click(screen.getByRole('button', { name: 'Load version' }))
    await waitFor(() => expect(load).toHaveBeenCalledWith('20260928T120227Z-session-start.csv'))
  })
})
