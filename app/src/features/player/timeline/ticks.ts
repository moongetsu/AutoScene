import type { View } from "./viewport"

/** Steps between labelled ticks in frames, then in timecode seconds. */
const FRAMES = [1, 2, 5, 10]
const SECONDS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600]

/**
 * How a frame counts on the timecode and back (`timecodeCount` and
 * `frameAtCount`); the same number without drop-frame.
 */
export type Counting = { count: (f: number) => number; frame: (n: number) => number }

const PLAIN: Counting = { count: (f) => f, frame: (n) => n }

/**
 * Where the ruler's labelled ticks go: the finest step that leaves at least
 * `minPx` between labels, on round timecodes — whole timecode seconds count
 * `round(fps)` frames, so at 23.976 a tick reads 00:20:40:00, not the frame
 * at a round second of real time, and drop-frame steps over the labels it
 * skips (a minute tick reads 00:01:00;02). `frameAt`/`rel` map between
 * seconds from the first frame and frames.
 */
export function ticks(
  v: View,
  width: number,
  fps: number,
  frameAt: (t: number) => number,
  rel: (f: number) => number,
  counting: Counting = PLAIN,
  minPx = 96,
): { step: number; frames: number[] } {
  const base = Math.max(1, Math.round(fps))
  const pxPerFrame = width / Math.max(v.span, 1e-9) / Math.max(fps, 1e-9)
  const steps = [...FRAMES, ...SECONDS.map((s) => s * base)]
  const step = steps.find((s) => s * pxPerFrame >= minPx) ?? steps[steps.length - 1]
  const frames: number[] = []
  const last = frameAt(v.t0 + v.span)
  for (let n = Math.ceil(counting.count(frameAt(v.t0)) / step) * step; frames.length < 500; n += step) {
    const f = counting.frame(n)
    if (f > last) break
    // A label drop-frame skips: in frame steps it isn't there (the frame
    // after it would crowd its neighbour); a second or more ticks there.
    if (step < base && counting.count(f) !== n) continue
    if (rel(f) >= v.t0 - 1e-9 && f !== frames[frames.length - 1]) frames.push(f)
  }
  return { step, frames }
}
