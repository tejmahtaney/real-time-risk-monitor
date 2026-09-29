import '@testing-library/jest-dom/vitest'

import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

// `globals: false` in vite.config.ts means Testing Library's automatic
// `afterEach(cleanup)` registration (which only fires under a global `afterEach`)
// never runs, so every component test would otherwise leak into the next one.
afterEach(cleanup)
