import { describe, expect, it } from "vitest"
import type { Segment } from "@/bindings"
import { nextShotStart, prevShotStart, segmentIndexAt, shotAt, shotNumbers } from "./segments"

const s = (start: number, end: number, kind: Segment["kind"] = "shot"): Segment => ({ start, end, kind, tiny: false })
// Shot, dissolve, shot, black, shot.
const segs = [s(0, 10), s(10, 14, "dissolve"), s(14, 30), s(30, 34, "black"), s(34, 50)]

describe("segments", () => {
  it("finds the segment at a frame", () => {
    expect([0, 9, 10, 13, 14, 49].map((f) => segmentIndexAt(segs, f))).toEqual([0, 0, 1, 1, 2, 4])
    expect(segmentIndexAt(segs, 50)).toBe(-1)
    expect(segmentIndexAt([], 0)).toBe(-1)
  })

  it("numbers only shots", () => {
    expect(shotNumbers(segs)).toEqual([0, -1, 1, -1, 2])
  })

  it("jumps between shot starts, skipping transitions and black", () => {
    expect(nextShotStart(segs, 0)).toBe(14)
    expect(nextShotStart(segs, 14)).toBe(34)
    expect(nextShotStart(segs, 34)).toBeNull()
    expect(prevShotStart(segs, 20)).toBe(14)
    expect(prevShotStart(segs, 14)).toBe(0)
    expect(prevShotStart(segs, 0)).toBeNull()
  })

  it("marks the shot under the playhead, or the one before a transition", () => {
    expect(shotAt(segs, 20)).toEqual(segs[2])
    expect(shotAt(segs, 12)).toEqual(segs[0])
    expect(shotAt(segs, 31)).toEqual(segs[2])
    expect(shotAt([s(0, 5, "black"), s(5, 9)], 2)).toEqual(s(5, 9))
  })
})
