import type { ReactElement, ReactNode } from 'react'

/** A labelled placeholder for a panel built in a later part (positions table, risk
 * tiles, alerts pane and charts are Parts 15 and 16; see docs/ROADMAP.md). */
export function PlaceholderPanel({
  title,
  note,
  className = '',
  children,
}: {
  title: string
  note: string
  className?: string
  children?: ReactNode
}): ReactElement {
  return (
    <section
      className={`flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label={title}
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">{title}</h2>
      <p className="text-sm text-neutral">{note}</p>
      {children}
    </section>
  )
}
