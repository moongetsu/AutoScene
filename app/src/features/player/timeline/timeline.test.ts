import { describe, expect, it } from "vitest"
import { frameAtCount, timecode, timecodeCount } from "@/lib/timecode"
import { ticks } from "./ticks"
import { centerOn, clampView, fit, follow, panBy, tAt, xOf, zoomAt } from "./viewport"

const LENGTH = 2700 // a 45-minute episode
const MIN = 1

describe("the timeline's view", () => {
  it("zooms around the pointer: the time under it stays put", () => {
    const v = fit(LENGTH)
    const at = 600
    const x = xOf(v, at, 1000)
    const z = zoomAt(v, 4, at, LENGTH, MIN)
    expect(z.span).toBeCloseTo(LENGTH / 4)
    expect(xOf(z, at, 1000)).toBeCloseTo(x)
    expect(tAt(z, x, 1000)).toBeCloseTo(at)
  })

  it("stays inside the episode and no closer than the smallest span", () => {
    const v = zoomAt(fit(LENGTH), 1e9, 5, LENGTH, MIN)
    expect(v.span).toBe(MIN)
    expect(zoomAt(fit(LENGTH), 0.1, 100, LENGTH, MIN)).toEqual(fit(LENGTH))
    expect(panBy({ t0: 10, span: 100 }, -50, LENGTH).t0).toBe(0)
    expect(panBy({ t0: 2650, span: 100 }, 80, LENGTH).t0).toBe(2600)
    expect(clampView({ t0: -5, span: 10_000 }, LENGTH, MIN)).toEqual(fit(LENGTH))
  })

  it("turns the page the way the playhead runs off it, and centres on a click", () => {
    const v = { t0: 100, span: 60 }
    expect(follow(v, 130, 129.9, LENGTH)).toBe(v)
    // Forwards: the next page starts just before the playhead.
    expect(follow(v, 160.1, 159.9, LENGTH)).toEqual({ t0: 157.1, span: 60 })
    // Backwards: the page before ends just after it, so what comes next shows.
    expect(follow(v, 99.9, 100.1, LENGTH).t0).toBeCloseTo(42.9)
    // A jump (Home, the next shot far away) centres it.
    expect(follow(v, 20, 130, LENGTH)).toEqual({ t0: 0, span: 60 })
    expect(follow(v, 1000, 130, LENGTH)).toEqual({ t0: 970, span: 60 })
    expect(centerOn(v, 1000, LENGTH)).toEqual({ t0: 970, span: 60 })
  })

  it("leaves a view the user moved away from the playing playhead", () => {
    // Scrolled ahead while it plays at 130: it stays out of view, frame after frame.
    const away = { t0: 500, span: 60 }
    expect(follow(away, 130.04, 130, LENGTH)).toBe(away)
    expect(follow(away, 130.08, 130.04, LENGTH)).toBe(away)
    // Until it runs into the view and off it again.
    expect(follow(away, 560.04, 559.99, LENGTH).t0).toBeCloseTo(557.04)
  })
})

describe("the ruler", () => {
  it("labels round timecodes far enough apart, frames when zoomed far in", () => {
    const at = (fps: number) => [(t: number) => Math.floor(t * fps + 1e-6), (f: number) => f / fps] as const
    // The whole episode on 1000 px: every 5 minutes (7200 frames at 24).
    expect(ticks(fit(LENGTH), 1000, 24, ...at(24)).step).toBe(7200)
    // Two seconds on 1000 px at 24 fps: every 5 frames, from the first in view.
    const close = ticks({ t0: 10, span: 2 }, 1000, 24, ...at(24))
    expect(close.step).toBe(5)
    expect(close.frames[0]).toBe(240)
    expect(close.frames.every((f, i) => i === 0 || f - close.frames[i - 1] === 5)).toBe(true)
    // At 23.976 a timecode second is 24 frames: ticks land on round timecodes.
    const ntsc = ticks({ t0: 1240, span: 60 }, 1000, 24000 / 1001, ...at(24000 / 1001))
    expect(ntsc.step).toBe(240)
    expect(ntsc.frames.every((f) => f % 240 === 0)).toBe(true)
  })

  it("reads round drop-frame timecodes at 29.97", () => {
    const fps = { num: 30000, den: 1001 }
    const counting = { count: (f: number) => timecodeCount(f, fps, true), frame: (n: number) => frameAtCount(n, fps, true) }
    const r = 30000 / 1001
    const frameAt = (t: number) => Math.floor(t * r + 1e-6)
    const rel = (f: number) => f / r
    const label = (f: number) => timecode(f, fps, true)
    // Seconds around the 2-minute mark: every label ends in ;00, but the
    // minute's first label is ;02 (;00 and ;01 don't exist there).
    const near = ticks({ t0: 110, span: 20 }, 1000, r, frameAt, rel, counting)
    expect(near.step).toBe(60)
    expect(near.frames.map(label)).toContain("00:02:00;02")
    expect(near.frames.map(label).every((l) => l.endsWith(";00") || l === "00:02:00;02")).toBe(true)
    // Frame steps leave out the labels drop-frame skips: no tick crowds the next.
    const close = ticks({ t0: rel(3596), span: 0.2 }, 1000, r, frameAt, rel, counting)
    expect(new Set(close.frames).size).toBe(close.frames.length)
    const fps60 = { num: 60000, den: 1001 }
    const r60 = 60000 / 1001
    const counting60 = { count: (f: number) => timecodeCount(f, fps60, true), frame: (n: number) => frameAtCount(n, fps60, true) }
    const frameAt60 = (t: number) => Math.floor(t * r60 + 1e-6)
    const rel60 = (f: number) => f / r60
    const minute = ticks({ t0: 59.9, span: 0.6 }, 1000, r60, frameAt60, rel60, counting60)
    expect(minute.step).toBe(5)
    expect(minute.frames.every((f, i) => i === 0 || f - minute.frames[i - 1] >= 5)).toBe(true)
    expect(minute.frames.map((f) => timecode(f, fps60, true))).toContain("00:01:00;05")
    expect(minute.frames.map((f) => timecode(f, fps60, true))).not.toContain("00:01:00;04")
    // The whole hour, by minutes: the tenth reads ;00, the others ;02.
    const hour = ticks({ t0: 0, span: 3600 }, 1000, r, frameAt, rel, counting)
    expect(hour.frames.map(label).slice(0, 3)).toEqual(["00:00:00;00", "00:10:00;00", "00:20:00;00"])
  })
})
