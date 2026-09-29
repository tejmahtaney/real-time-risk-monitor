import type { ReactElement } from 'react'

import type { ConnectionStatus } from '../store/riskStore'

const LABELS: Record<ConnectionStatus, string> = {
  connecting: 'Connecting…',
  open: 'Live',
  polling: 'Reconnecting (polling)',
  closed: 'Disconnected',
}

const DOT_CLASS: Record<ConnectionStatus, string> = {
  connecting: 'bg-warning',
  open: 'bg-up',
  polling: 'bg-breach',
  closed: 'bg-breach',
}

/** The header's connection-status dot: green while the WebSocket is open, red
 * otherwise (connecting, polling fallback, or closed). */
export function ConnectionDot({ status }: { status: ConnectionStatus }): ReactElement {
  return (
    <span className="flex items-center gap-2" data-testid="connection-dot" data-status={status}>
      <span
        aria-hidden="true"
        className={`inline-block h-2.5 w-2.5 rounded-full ${DOT_CLASS[status]}`}
      />
      <span className="text-sm text-neutral">{LABELS[status]}</span>
    </span>
  )
}
