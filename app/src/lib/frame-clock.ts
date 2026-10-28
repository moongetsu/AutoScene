import type { FrameMap } from "@/bindings"

/**
 * Maps between `<video>` time and source frames for one media, using the
 * decoder-truth frame index from the backend (see `FrameMap` in bindings).
 *
 * - Seeking goes to the **middle** of a frame's display interval, so float
 *   rounding in the browser can never land on a neighbour.
 * - The displayed frame is read from `requestVideoFrameCallback`'s
 *   `mediaTime` (the shown frame's own timestamp), with a quarter-frame
 *   tolerance — the same rule the Rust `FrameIndex::frame_at` uses.
 */
export class FrameClock {
  readonly frames: number
  /**
   * Frames a second, for what runs by the clock rather than by the picture
   * (playing backwards): the step's for constant-rate media, the map's
   * nominal rate otherwise — one frame of variable-rate media can last far
   * longer or shorter than the rest.
   */
  readonly fps: number
  private readonly offset: number
  private readonly step: number | null
  private readonly times: number[] | null

  constructor(map: Pick<FrameMap, "frames" | "fps" | "offset" | "step" | "times">) {
    this.frames = map.frames
    this.offset = map.offset ?? 0
    this.step = map.times ? null : (map.step ?? 0)
    this.times = map.times ? checkedTimes(map.times, map.frames) : null
    if (!this.times && !this.step) throw new Error("frame map has neither times nor step")
    const nominal = map.fps.num / map.fps.den
    this.fps = this.step
      ? 1 / this.step
      : Number.isFinite(nominal) && nominal > 0
        ? nominal
        : this.frames / Math.max(this.length, 1e-6)
  }

  /** Start of frame `f` relative to frame 0 (`f === frames` gives the end). */
  rel(f: number): number {
    const c = Math.max(0, Math.min(this.frames, f))
    return this.times ? this.times[c] : c * this.step!
  }

  /** Player time at which frame `f` starts. */
  start(f: number): number {
    return this.offset + this.rel(f)
  }

  duration(f: number): number {
    return this.rel(f + 1) - this.rel(f)
  }

  /** Where to seek the player to show frame `f`. */
  seekTime(f: number): number {
    const c = this.clamp(f)
    return this.offset + 0.5 * (this.rel(c) + this.rel(c + 1))
  }

  /**
   * Frame shown at player time `t`: the last frame starting at or before `t`,
   * or the next one when `t` is within a quarter frame of its start (players
   * report timestamps rounded to the container's timescale).
   */
  frameAt(t: number): number {
    const rel = t - this.offset
    if (rel <= 0) return 0
    if (!this.times) return this.clamp(Math.floor(rel / this.step! + 0.25))
    let lo = 0
    let hi = this.frames - 1
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1
      if (this.times[mid] <= rel) lo = mid
      else hi = mid - 1
    }
    const next = lo + 1
    return next < this.frames && this.times[next] - rel < 0.25 * this.duration(lo) ? next : lo
  }

  /** Total length in seconds. */
  get length(): number {
    return this.rel(this.frames)
  }

  clamp(f: number): number {
    return Math.max(0, Math.min(this.frames - 1, Math.round(f)))
  }
}

/**
 * A variable-rate map's frame starts, as they must be: one per frame and one
 * for the end, each a number later than the one before. The backend builds
 * them from a checked index, so anything else is a bug there; it fails here
 * rather than seek to wrong frames (a missing time read as 0 would send the
 * search anywhere).
 */
function checkedTimes(times: (number | null)[], frames: number): number[] {
  if (times.length !== frames + 1) throw new Error(`frame map has ${times.length} times for ${frames} frames`)
  let before = -Infinity
  for (const [f, t] of times.entries()) {
    if (t == null || !Number.isFinite(t) || t <= before) throw new Error(`frame map time ${f} is ${t}, after ${before}`)
    before = t
  }
  return times as number[]
}
