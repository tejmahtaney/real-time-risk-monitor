import type { ReactElement } from 'react'
import { NavLink } from 'react-router-dom'

const LINK_CLASS = ({ isActive }: { isActive: boolean }): string =>
  `rounded px-3 py-1 text-sm ${isActive ? 'bg-info/20 font-medium text-info' : 'text-neutral hover:bg-surface'}`

const LINKS: Array<{ to: string; label: string; end?: boolean }> = [
  { to: '/', label: 'Live', end: true },
  { to: '/backtest', label: 'Backtest' },
  { to: '/history', label: 'History' },
  { to: '/portfolios', label: 'Portfolios' },
  { to: '/data', label: 'Data' },
]

/** The header's page links: Live, Backtest, History, Portfolios and Data. */
export function ModeSwitch(): ReactElement {
  return (
    <nav aria-label="Pages" className="flex gap-1 rounded border border-border p-0.5">
      {LINKS.map(({ to, label, end }) => (
        <NavLink key={to} to={to} end={end} className={LINK_CLASS}>
          {label}
        </NavLink>
      ))}
    </nav>
  )
}
