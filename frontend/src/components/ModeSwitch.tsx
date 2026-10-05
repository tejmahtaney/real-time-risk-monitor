import type { ReactElement } from 'react'
import { NavLink } from 'react-router-dom'

const LINK_CLASS = ({ isActive }: { isActive: boolean }): string =>
  `rounded px-3 py-1 text-sm ${isActive ? 'bg-info/20 font-medium text-info' : 'text-neutral hover:bg-surface'}`

/** The header's Live and Backtest switch. */
export function ModeSwitch(): ReactElement {
  return (
    <nav aria-label="Mode" className="flex gap-1 rounded border border-border p-0.5">
      <NavLink to="/" end className={LINK_CLASS}>
        Live
      </NavLink>
      <NavLink to="/backtest" className={LINK_CLASS}>
        Backtest
      </NavLink>
    </nav>
  )
}
