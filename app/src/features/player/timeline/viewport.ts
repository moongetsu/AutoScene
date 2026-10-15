/**
 * The part of an episode the timeline shows: from `t0`, `span` seconds
 * (both measured from the episode's first frame, so VFR media zoom in real
 * time). Pure: the timeline keeps one and asks these for the next.
 */
export type View = { t0: number; span: number }

/** The whole episode. */
export function fit(length: number): View {
  return { t0: 0, span: Math.max(length, 1e-6) }
}

/** Inside the episode, no narrower than `minSpan` and no wider than it. */
export function clampView(v: View, length: number, minSpan: number): View {
  const span = Math.min(Math.max(v.span, Math.min(minSpan, length)), Math.max(length, 1e-6))
  const t0 = Math.min(Math.max(v.t0, 0), Math.max(0, length - span))
  return { t0, span }
}

/** Zoom by `factor` (> 1 closer) keeping time `at` where it is on screen. */
export function zoomAt(v: View, factor: number, at: number, length: number, minSpan: number): View {
  const span = v.span / factor
  const share = v.span > 0 ? (at - v.t0) / v.span : 0.5
  return clampView({ t0: at - share * span, span }, length, minSpan)
}

/** Move the view by `dt` seconds. */
export function panBy(v: View, dt: number, length: number): View {
  return clampView({ t0: v.t0 + dt, span: v.span }, length, 0)
}

/**
 * Keep the playhead in view the way a playing timeline does, as it moves
 * from `prev` to `t`: when it runs off the page, turn to the next one in the
 * direction it runs (forwards it starts just before the playhead, backwards
 * it ends just after it); when it jumps, centre it. A playhead the user
 * scrolled or zoomed away from stays out of view until it comes back.
 */
export function follow(v: View, t: number, prev: number, length: number): View {
  const inView = (x: number) => x >= v.t0 && x <= v.t0 + v.span
  if (inView(t)) return v
  if (Math.abs(t - prev) > v.span / 2) return centerOn(v, t, length)
  if (!inView(prev)) return v
  const lead = v.span * 0.05
  return clampView({ t0: t < prev ? t + lead - v.span : t - lead, span: v.span }, length, 0)
}

/** Centre the view on `t` (a click in the overview). */
export function centerOn(v: View, t: number, length: number): View {
  return clampView({ t0: t - v.span / 2, span: v.span }, length, 0)
}

export function xOf(v: View, t: number, width: number): number {
  return ((t - v.t0) / v.span) * width
}

export function tAt(v: View, x: number, width: number): number {
  return v.t0 + (x / Math.max(1, width)) * v.span
}
