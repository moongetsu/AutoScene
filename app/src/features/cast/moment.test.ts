import { describe, expect, it } from "vitest"
import type { MediaItem } from "@/bindings"
import { FrameClock } from "@/lib/frame-clock"
import { canPlayMoment, loopsBack, momentLoop } from "./moment"

describe("which faces can play their moment", () => {
  const media = (status: MediaItem["status"], proxy: MediaItem["proxy"]) => ({ status, proxy })

  it("plays episodes that need no preview copy, or whose copy is done", () => {
    expect(canPlayMoment(media("ready", "not_needed"))).toBe(true)
    expect(canPlayMoment(media("ready", "ready"))).toBe(true)
  })

  it("keeps the still while a preview copy is missing, being made or failed", () => {
    for (const proxy of ["unknown", "pending", "building", "failed"] as const) {
      expect(canPlayMoment(media("ready", proxy))).toBe(false)
    }
  })

  it("keeps the still for episodes that aren't indexed, or are gone", () => {
    for (const status of ["queued", "indexing", "failed", "missing"] as const) {
      expect(canPlayMoment(media(status, "not_needed"))).toBe(false)
    }
    expect(canPlayMoment(undefined)).toBe(false)
  })
})

describe("a moment's loop", () => {
  // 24 fps, its first frame half a second into the file (as proxies start).
  const cfr = new FrameClock({ frames: 240, fps: { num: 24, den: 1 }, offset: 0.5, step: 1 / 24, times: null })
  // Variable rate: frames of 0.1 s, then of 0.05 s from frame 5.
  const times = [0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75]
  const vfr = new FrameClock({ frames: 10, fps: { num: 15, den: 1 }, offset: null, step: null, times })

  it("starts in the middle of the track's first frame and ends on its last", () => {
    const loop = momentLoop(cfr, 48, 72)!
    expect(loop.first).toBe(48)
    expect(loop.last).toBe(71)
    expect(loop.from).toBeCloseTo(0.5 + 48.5 / 24)
    expect(cfr.frameAt(loop.from)).toBe(48)
  })

  it("follows a variable-rate episode's own frame times", () => {
    const loop = momentLoop(vfr, 4, 8)!
    expect(loop).toMatchObject({ first: 4, last: 7 })
    expect(loop.from).toBeCloseTo(0.45)
  })

  it("stays inside the episode", () => {
    expect(momentLoop(cfr, 230, 300)).toMatchObject({ first: 230, last: 239 })
    expect(momentLoop(cfr, -3, 2)).toMatchObject({ first: 0, last: 1 })
    expect(momentLoop(cfr, 240, 260)).toBeNull()
    expect(momentLoop(cfr, 10, 10)).toBeNull()
  })

  it("goes round again once its last frame shows, never later", () => {
    const loop = momentLoop(cfr, 48, 72)!
    expect(loopsBack(cfr, loop, cfr.start(70))).toBe(false)
    expect(loopsBack(cfr, loop, cfr.start(71))).toBe(true)
    expect(loopsBack(cfr, loop, cfr.start(80))).toBe(true)
    // Before the first seek lands, the file's opening frame shows.
    expect(loopsBack(cfr, loop, 0)).toBe(false)
    const v = momentLoop(vfr, 4, 8)!
    expect(loopsBack(vfr, v, 0.58)).toBe(false)
    expect(loopsBack(vfr, v, 0.6)).toBe(true)
  })

  it("holds a one-frame moment still", () => {
    const loop = momentLoop(cfr, 48, 49)!
    expect(loopsBack(cfr, loop, cfr.start(48))).toBe(false)
  })
})
