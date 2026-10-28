import { describe, expect, it } from "vitest"
import { FrameClock } from "./frame-clock"
import { formatFps, parseTimecode, timecode } from "./timecode"

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
    const c = new FrameClock({ frames: 1000, fps: { num: 24000, den: 1001 }, offset: 0.021, step, times: null })
    for (let f = 0; f < 1000; f++) {
      expect(c.frameAt(c.seekTime(f))).toBe(f)
      // rVFC reports the shown frame's own timestamp.
      expect(c.frameAt(c.start(f))).toBe(f)
      expect(c.frameAt(c.start(f) + 1e-4)).toBe(f)
    }
    expect(c.frameAt(-5)).toBe(0)
    expect(c.frameAt(1e9)).toBe(999)
    expect(c.length).toBeCloseTo(1000 * step)
    expect(c.fps).toBeCloseTo(24000 / 1001)
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
    const c = new FrameClock({ frames: times.length - 1, fps: { num: 60, den: 1 }, offset: 0, step: null, times })
    for (let f = 0; f < c.frames; f++) {
      expect(c.frameAt(c.seekTime(f))).toBe(f)
      expect(c.frameAt(times[f])).toBe(f)
    }
    expect(c.duration(50)).toBeCloseTo(1 / 60)
  })

  it("plays variable-rate media at its nominal rate, whatever frame is on screen", () => {
    // 25 fps with a two-second frame (a gap in a recording) and a frame one
    // tick of 1/90000 long (a repaired timestamp).
    const times = [0, 0.04, 2.04, 2.04 + 1 / 90000, 2.08, 2.12]
    const c = new FrameClock({ frames: times.length - 1, fps: { num: 25, den: 1 }, offset: 0, step: null, times })
    expect(c.duration(1)).toBeCloseTo(2)
    expect(c.duration(2)).toBeCloseTo(1 / 90000)
    expect(c.fps).toBe(25)
    // A map without a usable rate plays at its frames' average.
    const unrated = new FrameClock({ frames: 2, fps: { num: 0, den: 1 }, offset: 0, step: null, times: [0, 0.5, 1] })
    expect(unrated.fps).toBe(2)
  })

  it("refuses frame times that are missing, out of order or too few", () => {
    const map = (times: (number | null)[], frames = times.length - 1) =>
      new FrameClock({ frames, fps: { num: 25, den: 1 }, offset: 0, step: null, times })
    expect(() => map([0, 0.04, 0.08])).not.toThrow()
    // JSON carries NaN and infinity as null.
    expect(() => map([0, null, 0.08])).toThrow(/time 1/)
    expect(() => map([0, 0.04, 0.04])).toThrow(/time 2/)
    expect(() => map([0, 0.08, 0.04])).toThrow(/time 2/)
    expect(() => map([0, Number.NaN, 0.08])).toThrow(/time 1/)
    expect(() => map([0, 0.04, 0.08], 3)).toThrow(/3 times for 3 frames/)
  })
})

describe("parseTimecode", () => {
  const ntsc = { num: 30000, den: 1001 }
  const film = { num: 24000, den: 1001 }

  it("inverts timecode() exactly, drop-frame included", () => {
    for (const fps of [film, ntsc, { num: 60000, den: 1001 }, { num: 25, den: 1 }]) {
      for (const df of [false, true]) {
        for (const f of [0, 1, 2, 1799, 1800, 1801, 17981, 17982, 107892, 123456]) {
          expect(parseTimecode(timecode(f, fps, df), fps, df)).toBe(f)
        }
      }
    }
  })

  it("accepts frame numbers, jumps and short timecodes", () => {
    expect(parseTimecode("17099", film, false)).toBe(17099)
    expect(parseTimecode("+48", film, false, 100)).toBe(148)
    expect(parseTimecode("-12", film, false, 100)).toBe(88)
    expect(parseTimecode("01:00", film, false)).toBe(24)
    expect(parseTimecode("1:02:03", film, false)).toBe((62 * 24 + 3))
    expect(parseTimecode("00:11:52:11", film, false)).toBe(17099)
  })

  it("rejects nonsense", () => {
    expect(parseTimecode("", film, false)).toBeNull()
    expect(parseTimecode("abc", film, false)).toBeNull()
    expect(parseTimecode("00:00:00:24", film, false)).toBeNull()
    expect(parseTimecode("00:61:00:00", film, false)).toBeNull()
  })
})
