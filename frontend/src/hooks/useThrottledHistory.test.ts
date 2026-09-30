import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import { useRiskStore } from '../store/riskStore'
import { useThrottledHistory } from './useThrottledHistory'

function point(seq: number): HistoryPoint {
  return {
    seq,
    sim_time: new Date(seq * 1000).toISOString(),
    dollar_delta: seq,
    dollar_gamma_1pct: 0,
    vega: 0,
    theta: 0,
    gross_exposure: 0,
    market_value: 0,
    pnl_today: { delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, higher_order: 0, dividends: 0, residual: 0, actual: 0 },
    worst_limit_utilisation: 0,
    var_99: null,
  }
}

describe('useThrottledHistory', () => {
  beforeEach(() => {
    useRiskStore.getState().reset()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('starts with whatever is already in the store', () => {
    useRiskStore.getState().mergeHistory([point(1)])
    const { result } = renderHook(() => useThrottledHistory(500))
    expect(result.current.map((p) => p.seq)).toEqual([1])
  })

  it('does not pick up a store update until the interval elapses', () => {
    const { result } = renderHook(() => useThrottledHistory(500))
    act(() => {
      useRiskStore.getState().mergeHistory([point(1)])
    })
    expect(result.current).toEqual([])
  })

  it('picks up every update made within one window, the next time the interval fires', () => {
    const { result } = renderHook(() => useThrottledHistory(500))
    act(() => {
      useRiskStore.getState().mergeHistory([point(1)])
      useRiskStore.getState().mergeHistory([point(2)])
      useRiskStore.getState().mergeHistory([point(3)])
    })
    act(() => {
      vi.advanceTimersByTime(500)
    })
    expect(result.current.map((p) => p.seq)).toEqual([1, 2, 3])
  })

  it('does not re-render again if nothing changed since the last tick', () => {
    const { result } = renderHook(() => useThrottledHistory(500))
    act(() => {
      useRiskStore.getState().mergeHistory([point(1)])
    })
    act(() => {
      vi.advanceTimersByTime(500)
    })
    const afterFirstTick = result.current
    act(() => {
      vi.advanceTimersByTime(1_000)
    })
    expect(result.current).toBe(afterFirstTick)
  })
})
