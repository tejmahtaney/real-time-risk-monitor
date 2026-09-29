import '@testing-library/jest-dom/vitest'

import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

// `globals: false` in vite.config.ts means Testing Library's automatic
// `afterEach(cleanup)` registration (which only fires under a global `afterEach`)
// never runs, so every component test would otherwise leak into the next one.
afterEach(cleanup)

// jsdom (Part 16) has no `ResizeObserver`, and Recharts' `ResponsiveContainer` reads
// an element's `offsetWidth`/`offsetHeight` to size its chart — both are always 0 in
// jsdom, which makes `ResponsiveContainer` render nothing at all. A stub observer plus
// a fixed, non-zero size gives every chart test a real container to lay out into.
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}
const globalWithResizeObserver = globalThis as unknown as { ResizeObserver?: typeof ResizeObserverStub }
globalWithResizeObserver.ResizeObserver ??= ResizeObserverStub

Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, value: 400 })
Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, value: 300 })
