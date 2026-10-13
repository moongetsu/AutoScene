import { describe, expect, it } from "vitest"
import { FrameClock } from "./frame-clock"
import { formatFps, timecode } from "./timecode"

describe("timecode", () => {
  it("matches the Rust implementation", () => {
    expect(timecode(24 * 3600 + 24 * 61 + 5, { num: 24, den: 1 }, false)).toBe("01:01:01:05")
    // 29.97 DF: frame 1800 is 00:01:00;02 (frames 00 and 01 are dropped).
    expect(timecode(1800, { num: 30000, den: 1001 }, true)).toBe("00:01:00;02")
    // Every 10th minute keeps its frames.
    expect(timecode(17982, { num: 30000, den: 1001 }, true)).toBe("00:10:00;00")
    // 23.976 never drops frames.
    expect(timecode(100, { num: 24000, den: 1001 }, true)).toBe("00:00:04:04")
  })

  it("formats frame rates like editors do", () => {
    expect(formatFps({ num: 24000, den: 1001 })).toBe("23.976")
    expect(formatFps({ num: 25, den: 1 })).toBe("25")
    expect(formatFps({ num: 30000, den: 1001 })).toBe("29.97")
  })
})

describe("FrameClock", () => {
  it("round-trips every frame of a constant-rate map", () => {
    const step = 1001 / 24000
    const c = new FrameClock({ frames: 1000, offset: 0.021, step, times: null })
    for (let f = 0; f < 1000; f++) {
      expect(c.frameAt(c.seekTime(f))).toBe(f)
      // rVFC reports the shown frame's own timestamp.
      expect(c.frameAt(c.start(f))).toBe(f)
      expect(c.frameAt(c.start(f) + 1e-4)).toBe(f)
    }
    expect(c.frameAt(-5)).toBe(0)
    expect(c.frameAt(1e9)).toBe(999)
    expect(c.length).toBeCloseTo(1000 * step)
  })

  it("resolves variable-rate maps by search", () => {
    // 24 fps, then 60 fps, then 30 fps.
    const times: number[] = []
    let t = 0
    for (const [n, d] of [
      [48, 1 / 24],
      [120, 1 / 60],
      [60, 1 / 30],
    ]) {
      for (let i = 0; i < n; i++) {
        times.push(t)
        t += d
      }
    }
    times.push(t)
    const c = new FrameClock({ frames: times.length - 1, offset: 0, step: null, times })
    for (let f = 0; f < c.frames; f++) {
      expect(c.frameAt(c.seekTime(f))).toBe(f)
      expect(c.frameAt(times[f])).toBe(f)
    }
    expect(c.duration(50)).toBeCloseTo(1 / 60)
  })
})
