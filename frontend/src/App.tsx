import { useEffect, type ReactElement } from 'react'

import { startStream } from './api/stream'
import { Dashboard } from './components/Dashboard'
import { Header } from './components/Header'

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
        <Dashboard />
      </main>
    </div>
  )
}
