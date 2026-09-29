import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { api, ApiError } from '../api/rest'
import { Controls } from './Controls'

vi.mock('../api/rest', async () => {
  const actual = await vi.importActual<typeof import('../api/rest')>('../api/rest')
  return { ...actual, api: { ...actual.api, addPosition: vi.fn(), pause: vi.fn(), resume: vi.fn(), setSpeed: vi.fn() } }
})

afterEach(() => {
  vi.clearAllMocks()
})

describe('Controls: add-position form validation', () => {
  it('shows a field error and does not submit when the strike is malformed', async () => {
    const user = userEvent.setup()
    render(<Controls paused={false} clockSpeed={60} />)

    await user.type(screen.getByPlaceholderText('AAPL'), 'AAPL')
    await user.selectOptions(screen.getByRole('combobox'), 'call')
    await user.type(screen.getByPlaceholderText('ATM+5%'), 'not-a-strike')
    await user.type(screen.getByPlaceholderText('+30d'), '+30d')
    await user.type(screen.getByPlaceholderText('10'), '5')
    await user.click(screen.getByRole('button', { name: /add position/i }))

    expect(await screen.findByText(/expected a number, atm/i)).toBeInTheDocument()
    expect(api.addPosition).not.toHaveBeenCalled()
  })

  it('submits a well-formed row and shows a server error inline on failure', async () => {
    vi.mocked(api.addPosition).mockRejectedValueOnce(new ApiError(422, 'strike must be positive'))
    const user = userEvent.setup()
    render(<Controls paused={false} clockSpeed={60} />)

    await user.type(screen.getByPlaceholderText('AAPL'), 'AAPL')
    await user.selectOptions(screen.getByRole('combobox'), 'call')
    await user.type(screen.getByPlaceholderText('ATM+5%'), 'ATM+5%')
    await user.type(screen.getByPlaceholderText('+30d'), '+30d')
    await user.type(screen.getByPlaceholderText('10'), '5')
    await user.click(screen.getByRole('button', { name: /add position/i }))

    expect(api.addPosition).toHaveBeenCalledOnce()
    expect(await screen.findByTestId('add-position-server-error')).toHaveTextContent(
      'strike must be positive',
    )
  })
})

describe('Controls: speed selector', () => {
  it('highlights the current speed', () => {
    render(<Controls paused={false} clockSpeed={120} />)
    expect(screen.getByRole('button', { name: '120×' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: '60×' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('calls setSpeed when a speed is clicked', async () => {
    const user = userEvent.setup()
    render(<Controls paused={false} clockSpeed={60} />)
    await user.click(screen.getByRole('button', { name: '300×' }))
    expect(api.setSpeed).toHaveBeenCalledWith(300)
  })
})
