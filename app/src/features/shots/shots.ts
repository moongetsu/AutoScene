import type { CSSProperties } from "react"

/** What can be said about the bodies a shot without the character's face shows. */
export type BodyVerdict = "not_them" | "not_a_person"

/** Shot cards are 16:9. */
export const CARD_ASPECT = 16 / 9

/** The cell shown while the pointer is elsewhere: the middle of the shot. */
export function restingCell(cells: number): number {
  return Math.max(0, Math.floor((cells - 1) / 2))
}

/** The sprite cell under a pointer `x` pixels into a card `width` wide. */
export function cellAt(x: number, width: number, cells: number): number {
  if (cells <= 0 || width <= 0) return 0
  return Math.min(cells - 1, Math.max(0, Math.floor((x / width) * cells)))
}

/** "<0.1 s", "0.4 s", "2.5 s", "12 s", "1:05". */
export function formatShotLength(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return ""
  if (sec < 0.05) return "<0.1 s"
  if (sec < 10) return `${(Math.round(sec * 10) / 10).toFixed(1)} s`
  if (sec < 60) return `${Math.round(sec)} s`
  const s = Math.round(sec)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`
}

/**
 * Show cell `cell` of a sprite of `cells` cells, each `aspect` wide per unit
 * of height, centred in a card: wider cells fill its width (bars above and
 * below), taller ones (4:3) its height (bars at the sides) — never cropped.
 */
export function spriteStyle(cells: number, cell: number, aspect: number): CSSProperties {
  // The strip's centre sits on the card's; move it so cell `cell`'s does.
  const shift = ((cells / 2 - cell - 0.5) * 100) / cells
  // (A little narrower than 16:9 — rounded image sizes — still fills the width.)
  const fit = aspect >= CARD_ASPECT * 0.95 ? { width: `${cells * 100}%` } : { height: "100%" }
  return { ...fit, transform: `translate(calc(-50% + ${shift}%), -50%)` }
}
