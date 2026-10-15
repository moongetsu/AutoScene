import { describe, expect, it } from "vitest"
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

  it("turns the page when the playhead runs off it, and centres on a click", () => {
    const v = { t0: 100, span: 60 }
    expect(follow(v, 130, LENGTH)).toBe(v)
    expect(follow(v, 161, LENGTH)).toEqual({ t0: 158, span: 60 })
    expect(follow(v, 20, LENGTH).t0).toBe(17)
    expect(centerOn(v, 1000, LENGTH)).toEqual({ t0: 970, span: 60 })
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
})
