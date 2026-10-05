import { describe, expect, it } from 'vitest'

import { formatBytes, usageLevel } from './usage'

describe('usageLevel', () => {
  it('is ok below the warning level', () => {
    expect(usageLevel(799, 1000, 0.8)).toBe('ok')
  })
  it('warns at the warning level and at the limit', () => {
    expect(usageLevel(800, 1000, 0.8)).toBe('warning')
    expect(usageLevel(1000, 1000, 0.8)).toBe('warning')
  })
  it('is over above the limit', () => {
    expect(usageLevel(1001, 1000, 0.8)).toBe('over')
  })
})

describe('formatBytes', () => {
  it('uses binary units', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
  })
})
