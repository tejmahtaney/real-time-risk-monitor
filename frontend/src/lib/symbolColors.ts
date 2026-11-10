/**
 * One fixed colour per underlying symbol, shared by the allocation donut and the
 * positions table's group headers (Part 15), so SPY (say) is always the same colour
 * everywhere on the page.
 *
 * An 8-hue categorical palette, fixed order, validated against this app's dark
 * surface (`--color-surface`, `#111827`) with the dataviz skill's validator
 * (lightness band, chroma floor, CVD separation, normal-vision floor and contrast
 * all pass on the adjacent pairlist). The five
 * symbols `config/market.toml` actually calibrates get their own fixed slots so the
 * demo book never collides; any other symbol (added through the positions form)
 * falls back to a deterministic hash into the three remaining slots.
 */
const PALETTE = [
  '#3987e5', // blue
  '#d95926', // orange
  '#199e70', // aqua
  '#c98500', // yellow
  '#d55181', // magenta
  '#008300', // green
  '#9085e9', // violet
  '#e66767', // red
] as const

const KNOWN_SYMBOL_SLOTS: Record<string, number> = {
  AAPL: 0,
  MSFT: 1,
  NVDA: 2,
  SPY: 3,
  TSLA: 4,
}

// The three slots left over for anything not in `KNOWN_SYMBOL_SLOTS`, consecutive so
// a hash can land on one with a single modulo (see `symbolColor`).
const FALLBACK_SLOT_START = 5
const FALLBACK_SLOT_COUNT = PALETTE.length - FALLBACK_SLOT_START

function hashString(value: string): number {
  let hash = 0
  for (let i = 0; i < value.length; i++) {
    hash = (Math.imul(31, hash) + value.charCodeAt(i)) | 0
  }
  return hash >>> 0
}

/** Indexes `PALETTE` by a slot this module itself computed (always 0..7), so the
 * lookup can never actually miss — this just satisfies `noUncheckedIndexedAccess`
 * without spreading a `| undefined` for every caller to re-check. */
function paletteColor(slot: number): string {
  const colour: string | undefined = PALETTE[slot]
  return colour ?? PALETTE[0]
}

/** The fixed colour for `symbol`. Deterministic: the same symbol always maps to the
 * same hex, regardless of what other symbols are in the book at the time. */
export function symbolColor(symbol: string): string {
  const knownSlot = KNOWN_SYMBOL_SLOTS[symbol]
  const slot = knownSlot !== undefined ? knownSlot : FALLBACK_SLOT_START + (hashString(symbol) % FALLBACK_SLOT_COUNT)
  return paletteColor(slot)
}
