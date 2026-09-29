import { useEffect, type ReactElement } from 'react'

import { startStream } from './api/stream'
import { Header } from './components/Header'
import { PlaceholderPanel } from './components/PlaceholderPanel'

export function App(): ReactElement {
  useEffect(() => {
    // `startStream` is a no-op after the first call, so React StrictMode's double
    // mount in development opens exactly one WebSocket connection (see
    // `src/api/stream.ts`'s doc comment).
    startStream()
  }, [])

  return (
    <div className="flex min-h-screen flex-col">
      <Header />
      <main className="grid flex-1 grid-cols-1 gap-4 p-4 lg:grid-cols-3">
        <PlaceholderPanel
          title="Positions"
          note="The positions table lands in Part 15."
          className="lg:col-span-2"
        />
        <PlaceholderPanel title="Risk tiles" note="Limit-utilisation tiles land in Part 15." />
        <PlaceholderPanel
          title="Alerts"
          note="The alerts pane lands in Part 15."
          className="lg:col-span-2"
        />
        <PlaceholderPanel title="Charts" note="Greeks, allocation and P&L charts land in Part 16." />
      </main>
    </div>
  )
}
