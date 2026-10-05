import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { StorageUsageResponse } from '../../api/generated/StorageUsageResponse'
import { UsageBar } from './UsageBar'

const usage = (total: number): StorageUsageResponse => ({
  usage_bytes: { sessions: total, 'market-data': 0 },
  total_bytes: total,
  limit_bytes: 1000,
  warn_fraction: 0.8,
  warning: total >= 800,
  storage_path: '/data/storage',
})

describe('UsageBar', () => {
  it.each([
    [799, 'ok', 'bg-info'],
    [800, 'warning', 'bg-warning'],
    [1001, 'over', 'bg-breach'],
  ])('at %i bytes the fill is %s', (total, level, colour) => {
    render(<UsageBar usage={usage(total)} />)
    const fill = screen.getByTestId('usage-fill')
    expect(fill).toHaveAttribute('data-level', level)
    expect(fill).toHaveClass(colour)
  })

  it('shows the storage path and never draws past the limit', () => {
    render(<UsageBar usage={usage(5000)} />)
    expect(screen.getByText('/data/storage')).toBeInTheDocument()
    expect(screen.getByTestId('usage-fill')).toHaveStyle({ width: '100%' })
  })

  it('scales folder bars to the largest folder while the top bar stays against the limit', () => {
    const small: StorageUsageResponse = {
      ...usage(150),
      usage_bytes: { sessions: 100, 'market-data': 50 },
    }
    render(<UsageBar usage={small} />)
    const [largest, half] = screen.getAllByTestId('folder-fill')
    expect(largest).toHaveStyle({ width: '100%' })
    expect(half).toHaveStyle({ width: '50%' })
    expect(screen.getByTestId('usage-fill')).toHaveStyle({ width: '15%' })
  })
})
