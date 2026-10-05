/**
 * Holds a backtest run in the server's memory for as long as the backtest page is on screen.
 *
 * React's StrictMode mounts, unmounts and mounts every effect again in development. A plain
 * "on unmount, free the run" would therefore free it on the first, throw-away unmount, and the
 * second mount would find the run gone. So releasing is deferred by one task: if the same run is
 * taken again before it fires (the StrictMode re-mount), the release is cancelled. A real route
 * change takes no new lease, so the release fires. A page unload frees the run at once with a
 * keepalive `DELETE` (see `releaseBacktestOnUnload`).
 */
import { api, releaseBacktestOnUnload } from '../api/rest'

const pending = new Map<string, ReturnType<typeof setTimeout>>()
const held = new Set<string>()
let unloadHooked = false

function hookUnload(): void {
  if (unloadHooked || typeof window === 'undefined') return
  unloadHooked = true
  window.addEventListener('pagehide', () => {
    for (const id of held) releaseBacktestOnUnload(id)
  })
}

/** Takes a lease on `id`. The returned function gives it back; the run is freed on the
 * server only if nobody takes the same run again straight afterwards. */
export function leaseRun(id: string): () => void {
  hookUnload()
  const timer = pending.get(id)
  if (timer !== undefined) {
    clearTimeout(timer)
    pending.delete(id)
  }
  held.add(id)
  return () => {
    held.delete(id)
    pending.set(
      id,
      setTimeout(() => {
        pending.delete(id)
        api.stopBacktest(id).catch(() => undefined)
      }, 0),
    )
  }
}
