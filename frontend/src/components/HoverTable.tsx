import type { ReactElement, ReactNode } from 'react'

/** A small generic hover-reveal panel: shows `children` below `trigger` on hover or
 * keyboard focus. No existing component in this codebase does element-hover-with-rich-
 * content (only native `title` tooltips and Recharts' own chart-point tooltip), so
 * this is new and reusable wherever a compact value needs a richer breakdown on
 * demand — first used by the VaR tile's three-method table (Part 18). Pure Tailwind
 * `group`/`group-hover`, plus `group-focus-within` so a keyboard or touch user can
 * reach the content by focusing the trigger, not just by hovering it. */
export function HoverTable({
  trigger,
  children,
}: {
  trigger: ReactNode
  children: ReactNode
}): ReactElement {
  return (
    <div className="group relative inline-flex">
      {trigger}
      <div className="pointer-events-none invisible absolute left-1/2 top-full z-10 mt-2 -translate-x-1/2 rounded-lg border border-border bg-surface-raised p-2 opacity-0 shadow-lg transition-opacity group-hover:pointer-events-auto group-hover:visible group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:visible group-focus-within:opacity-100">
        {children}
      </div>
    </div>
  )
}
