import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { BacktestListItem } from '../../api/generated/BacktestListItem'
import { SetupView } from './SetupView'

const COVERAGE = {
  first_date: '2026-05-05',
  last_date: '2026-09-28',
  aligned_dates: 81,
  missing_symbols: [],
}

function item(id: string, overrides: Partial<BacktestListItem> = {}): BacktestListItem {
  return {
    id,
    state: 'finished',
    source: { kind: 'archive' },
    start_date: '2026-06-04',
    last_date: '2026-09-28',
    days: 80,
    total_pnl: 111_503,
    created: '2026-09-30T12:41:01Z',
    ...overrides,
  }
}

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status < 400, status, json: () => Promise.resolve(body) } as Response
}

let routes: Record<string, (init?: RequestInit) => Response>
let fetchMock: ReturnType<typeof vi.fn>

describe('SetupView', () => {
  beforeEach(() => {
    routes = {
      'GET /api/backtests/archive-coverage': () => jsonResponse(COVERAGE),
      'GET /api/backtests': () => jsonResponse([]),
    }
    fetchMock = vi.fn((path: string, init?: RequestInit) => {
      const handler = routes[`${init?.method ?? 'GET'} ${path}`]
      return Promise.resolve(
        handler ? handler(init) : jsonResponse({ error: { message: `no route ${path}` } }, 404),
      )
    })
    vi.stubGlobal('fetch', fetchMock)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('shows how many dates the archive covers and starts an archive run', async () => {
    routes['POST /api/backtests'] = () => jsonResponse({ id: 'run-1' }, 201)
    const onStarted = vi.fn()
    render(<SetupView onStarted={onStarted} onOpenSaved={vi.fn()} />)

    expect(
      await screen.findByText(/81 aligned dates, 2026-05-05 to 2026-09-28/),
    ).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Use archived real history' }))
    await waitFor(() => expect(onStarted).toHaveBeenCalledWith('run-1'))
    const post = fetchMock.mock.calls.find(
      (call) => (call[1] as RequestInit | undefined)?.method === 'POST',
    )
    expect(JSON.parse((post?.[1] as RequestInit).body as string)).toEqual({ source: 'archive' })
  })

  it('disables the archive button when nothing is archived', async () => {
    routes['GET /api/backtests/archive-coverage'] = () =>
      jsonResponse({
        first_date: null,
        last_date: null,
        aligned_dates: 0,
        missing_symbols: ['SPY'],
      })
    render(<SetupView onStarted={vi.fn()} onOpenSaved={vi.fn()} />)
    expect(await screen.findByText(/No archived history for SPY/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Use archived real history' })).toBeDisabled()
  })

  it('shows the sample format beside the upload area', () => {
    render(<SetupView onStarted={vi.fn()} onOpenSaved={vi.fn()} />)
    expect(screen.getByText(/date,symbol,close/)).toBeInTheDocument()
  })

  it('shows the server validation error with its line number for a bad upload', async () => {
    routes['POST /api/backtests'] = () =>
      jsonResponse({ error: { message: 'line 7: close "abc" is not a number' } }, 400)
    const onStarted = vi.fn()
    render(<SetupView onStarted={onStarted} onOpenSaved={vi.fn()} />)

    const file = new File(['date,symbol,close\n'], 'prices.csv', { type: 'text/csv' })
    fireEvent.drop(screen.getByTestId('drop-zone'), { dataTransfer: { files: [file] } })

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'line 7: close "abc" is not a number',
    )
    expect(onStarted).not.toHaveBeenCalled()
    const post = fetchMock.mock.calls.find(
      (call) => (call[1] as RequestInit | undefined)?.method === 'POST',
    )
    expect((post?.[1] as RequestInit).body).toBeInstanceOf(FormData)
  })

  it('lists saved runs newest first, marks unfinished ones, and opens one', async () => {
    routes['GET /api/backtests'] = () =>
      jsonResponse([
        item('old-run', { created: '2026-09-01T00:00:00Z' }),
        item('new-run', {
          created: '2026-09-30T00:00:00Z',
          state: 'unfinished',
          total_pnl: null,
          days: 12,
          source: { kind: 'upload', filename: 'mine.csv' },
        }),
      ])
    const onOpen = vi.fn()
    render(<SetupView onStarted={vi.fn()} onOpenSaved={onOpen} />)

    const rows = await screen.findAllByTestId(/^saved-run-/)
    expect(rows.map((row) => row.getAttribute('data-testid'))).toEqual([
      'saved-run-new-run',
      'saved-run-old-run',
    ])
    expect(rows[0]).toHaveTextContent('Unfinished')
    expect(rows[0]).toHaveTextContent('mine.csv')
    expect(rows[1]).toHaveTextContent('Archive')
    expect(rows[1]).toHaveTextContent('$111.5k')

    await userEvent.click(rows[1]!.querySelector('button')!)
    expect(onOpen).toHaveBeenCalledWith('old-run')
  })

  it('deletes a run only after its id is typed back, then refreshes the list', async () => {
    let deleted = false
    routes['GET /api/backtests'] = () => jsonResponse(deleted ? [] : [item('run-x')])
    routes['DELETE /api/backtests/run-x/files'] = (init) => {
      expect(JSON.parse(init?.body as string)).toEqual({ confirm: 'run-x' })
      deleted = true
      return { ok: true, status: 204 } as Response
    }
    render(<SetupView onStarted={vi.fn()} onOpenSaved={vi.fn()} />)

    await userEvent.click(await screen.findByRole('button', { name: 'Delete' }))
    const confirm = screen.getByRole('button', { name: 'Delete files' })
    expect(confirm).toBeDisabled()
    expect(
      fetchMock.mock.calls.some(
        (call) => (call[1] as RequestInit | undefined)?.method === 'DELETE',
      ),
    ).toBe(false)

    await userEvent.type(screen.getByLabelText('Type the run id to confirm'), 'run-')
    expect(confirm).toBeDisabled()
    await userEvent.type(screen.getByLabelText('Type the run id to confirm'), 'x')
    expect(confirm).toBeEnabled()
    await userEvent.click(confirm)

    await waitFor(() => expect(screen.getByText('No saved runs yet.')).toBeInTheDocument())
    expect(deleted).toBe(true)
  })

  it('cancelling the confirmation deletes nothing', async () => {
    routes['GET /api/backtests'] = () => jsonResponse([item('run-y')])
    render(<SetupView onStarted={vi.fn()} onOpenSaved={vi.fn()} />)
    await userEvent.click(await screen.findByRole('button', { name: 'Delete' }))
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(screen.queryByTestId('delete-confirm')).not.toBeInTheDocument()
    expect(
      fetchMock.mock.calls.some(
        (call) => (call[1] as RequestInit | undefined)?.method === 'DELETE',
      ),
    ).toBe(false)
  })
})
