/**
 * Chart colours as plain hex. Recharts passes these straight through as SVG
 * `stroke`/`fill` attributes, so raw hex is used here rather than `var(--color-*)`,
 * to avoid depending on custom-property resolution inside a third-party SVG
 * attribute. Values mirror the tokens in `index.css`'s `@theme` block.
 */
export const CHART_LINE = '#60a5fa' // a chart accent, distinct from the up/down/warning/breach status colours
export const CHART_GRID = '#374151' // --color-border
export const CHART_AXIS = '#9ca3af' // --color-neutral
export const CHART_LIMIT_LINE = '#9ca3af' // the dashed reference line: neutral, since the limit line itself isn't a live status
export const CHART_WARNING_BAND = '#fbbf24' // --color-warning, used at low opacity for the shaded band
export const CHART_TOOLTIP_BG = '#1f2937' // --color-surface-raised
export const CHART_TOOLTIP_BORDER = '#374151' // --color-border
