import type { Presence, Segment } from "@/bindings"
import { type View, xOf } from "./viewport"

/** The design tokens a canvas needs, read once from CSS (see styles/tokens.css). */
export type Palette = { inset: string; flash: string; accent: string; hatch: CanvasPattern | string }

let cached: Palette | null = null

export function palette(ctx: CanvasRenderingContext2D): Palette {
  if (cached) return cached
  const css = getComputedStyle(document.documentElement)
  const token = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  // Transitions are hatched, as in the shot strip before.
  const tile = document.createElement("canvas")
  tile.width = tile.height = 4
  const t = tile.getContext("2d")
  let hatch: CanvasPattern | string = "rgba(255,255,255,0.18)"
  if (t) {
    t.strokeStyle = "rgba(255,255,255,0.22)"
    t.lineWidth = 1.5
    t.beginPath()
    t.moveTo(0, 4)
    t.lineTo(4, 0)
    t.stroke()
    hatch = ctx.createPattern(tile, "repeat") ?? hatch
  }
  cached = { inset: token("--bg-inset", "#050506"), flash: token("--fg-2", "#b4b1ac"), accent: token("--accent", "#e0b45c"), hatch }
  return cached
}

/** A canvas sized for the screen it is on: drawn in CSS pixels. */
export function prepare(canvas: HTMLCanvasElement, width: number, height: number): CanvasRenderingContext2D | null {
  const dpr = window.devicePixelRatio || 1
  const w = Math.max(1, Math.round(width * dpr))
  const h = Math.max(1, Math.round(height * dpr))
  if (canvas.width !== w) canvas.width = w
  if (canvas.height !== h) canvas.height = h
  const ctx = canvas.getContext("2d")
  if (!ctx) return null
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, width, height)
  return ctx
}

/** The first of `items` (sorted by start) that may reach into time `t` (by `end`). */
function firstEndingAfter<T>(items: T[], end: (x: T) => number, t: number): number {
  let lo = 0
  let hi = items.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (end(items[mid]) < t) lo = mid + 1
    else hi = mid
  }
  return lo
}

/**
 * Shots as blocks taking turns in two greys, transitions hatched, black
 * frames dark, flashes bright; a hairline between them. Only what is in view.
 */
export function drawSegments(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  view: View,
  segments: Segment[],
  numbers: number[],
  rel: (f: number) => number,
) {
  const p = palette(ctx)
  const end = view.t0 + view.span
  for (let i = firstEndingAfter(segments, (s) => rel(s.end), view.t0); i < segments.length; i++) {
    const s = segments[i]
    const a = rel(s.start)
    if (a > end) break
    const x0 = xOf(view, a, width)
    const x1 = xOf(view, rel(s.end), width)
    ctx.fillStyle =
      s.kind === "shot"
        ? numbers[i] % 2 === 0
          ? "rgba(255,255,255,0.09)"
          : "rgba(255,255,255,0.15)"
        : s.kind === "black"
          ? p.inset
          : s.kind === "flash"
            ? p.flash
            : p.hatch
    ctx.fillRect(x0, 0, Math.max(1, x1 - x0), height)
    if (x1 - x0 > 3) {
      ctx.fillStyle = "rgba(0,0,0,0.5)"
      ctx.fillRect(x1 - 1, 0, 1, height)
    }
  }
}

/** A character's time on screen: rounded bars, gold when they are the focus. */
export function drawSpans(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  view: View,
  spans: Presence[],
  focused: boolean,
  rel: (f: number) => number,
) {
  const p = palette(ctx)
  const end = view.t0 + view.span
  const bar = 10
  const y = (height - bar) / 2
  ctx.fillStyle = focused ? p.accent : "rgba(255,255,255,0.4)"
  if (focused) {
    ctx.shadowColor = p.accent
    ctx.shadowBlur = 8
  }
  // Spans of different faces may overlap: all of them, those in view drawn.
  for (const s of spans) {
    const a = rel(s.start)
    if (a > end || rel(s.end) < view.t0) continue
    const x0 = xOf(view, a, width)
    const w = Math.max(2, xOf(view, rel(s.end), width) - x0)
    ctx.beginPath()
    ctx.roundRect(x0, y, w, bar, Math.min(bar / 2, w / 2))
    ctx.fill()
  }
  ctx.shadowBlur = 0
}
