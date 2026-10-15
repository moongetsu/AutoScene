import type { Segment } from "@/bindings"

/** Index of the segment containing frame `f` (segments cover the media without gaps). */
export function segmentIndexAt(segments: readonly Segment[], f: number): number {
  let lo = 0
  let hi = segments.length - 1
  if (hi < 0) return -1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (segments[mid].start <= f) lo = mid
    else hi = mid - 1
  }
  return f >= segments[lo].start && f < segments[lo].end ? lo : -1
}

/** Ordinal (0-based) of each shot among the shots, `-1` for other segments. */
export function shotNumbers(segments: readonly Segment[]): number[] {
  let n = 0
  return segments.map((s) => (s.kind === "shot" ? n++ : -1))
}

/**
 * Where "next cut" goes from frame `f`: the start of the next shot. Shots
 * are what editors jump between; transitions and black are skipped.
 */
export function nextShotStart(segments: readonly Segment[], f: number): number | null {
  for (const s of segments) if (s.kind === "shot" && s.start > f) return s.start
  return null
}

/** The start of the shot before the one at `f` (or of the current shot, when `f` is inside it). */
export function prevShotStart(segments: readonly Segment[], f: number): number | null {
  let best: number | null = null
  for (const s of segments) {
    if (s.start >= f) break
    if (s.kind === "shot") best = s.start
  }
  return best
}

/** The shot to mark at frame `f`: the shot containing it, else the nearest one before. */
export function shotAt(segments: readonly Segment[], f: number): Segment | null {
  const i = segmentIndexAt(segments, f)
  if (i < 0) return null
  for (let j = i; j >= 0; j--) if (segments[j].kind === "shot") return segments[j]
  for (let j = i + 1; j < segments.length; j++) if (segments[j].kind === "shot") return segments[j]
  return null
}
