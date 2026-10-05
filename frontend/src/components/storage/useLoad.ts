import { useCallback, useEffect, useState } from 'react'

/** Runs an async loader on mount (and whenever `deps` change) and keeps its result and error.
 * `reload` runs it again. A result that arrives after the component has moved on is dropped. */
export function useLoad<T>(
  load: () => Promise<T>,
  deps: ReadonlyArray<unknown>,
): { data: T | null; error: string | null; reload: () => void } {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [tick, setTick] = useState(0)
  const reload = useCallback(() => setTick((n) => n + 1), [])

  useEffect(() => {
    let cancelled = false
    load()
      .then((value) => {
        if (cancelled) return
        setData(value)
        setError(null)
      })
      .catch((err: unknown) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err))
      })
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the caller lists the real inputs in `deps`
  }, [...deps, tick])

  return { data, error, reload }
}
