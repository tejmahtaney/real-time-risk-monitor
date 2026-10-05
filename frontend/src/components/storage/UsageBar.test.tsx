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
})
