/**
 * Reads `history` from the risk store, but only lets a new value through at most
 * once every `intervalMs` (twice a second by default), regardless of how often the
 * store itself updates (up to once per engine tick). The Greeks-over-time charts
 * read this instead of `useHistory` directly, per the prompt: "update charts at
 * most twice a second ... so the page stays smooth."
 *
 * Subscribes to the store directly (not via a `useSyncExternalStore` selector),
 * because the whole point is to *not* re-render on every store change — a plain
 * selector hook re-renders exactly as often as the store updates, which is the
 * behaviour this hook exists to avoid.
 */
import { useEffect, useRef, useState } from 'react'

import type { HistoryPoint } from '../api/generated/HistoryPoint'
import { useRiskStore, type RiskStore } from '../store/riskStore'

const DEFAULT_INTERVAL_MS = 500

export function useThrottledHistory(
  intervalMs = DEFAULT_INTERVAL_MS,
  store: RiskStore = useRiskStore,
): HistoryPoint[] {
  const pendingRef = useRef<HistoryPoint[]>(store.getState().history)
  const dirtyRef = useRef(false)
  const [history, setHistory] = useState(pendingRef.current)

  useEffect(() => {
    const unsubscribe = store.subscribe((state) => {
      pendingRef.current = state.history
      dirtyRef.current = true
    })
    const timer = setInterval(() => {
      if (!dirtyRef.current) return
      dirtyRef.current = false
      setHistory(pendingRef.current)
    }, intervalMs)
    return () => {
      unsubscribe()
      clearInterval(timer)
    }
  }, [intervalMs, store])

  return history
}
