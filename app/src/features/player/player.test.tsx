import { act, cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { FrameStore, useFrame, useFrameSelector } from "./frame-store"
import { nextRate, rateLabel, reverseFrames } from "./shuttle"

afterEach(cleanup)

describe("J/K/L", () => {
  it("speeds up on each press, turns round, and stops on K", () => {
    const presses = (keys: ("j" | "k" | "l")[]) => keys.reduce((r, k) => nextRate(r, k), 0)
    expect(presses(["l"])).toBe(1)
    expect(presses(["l", "l"])).toBe(2)
    expect(presses(["l", "l", "l", "l"])).toBe(4)
    expect(presses(["j"])).toBe(-1)
    expect(presses(["j", "j", "j"])).toBe(-4)
    expect(presses(["l", "l", "j"])).toBe(-1)
    expect(presses(["l", "l", "k"])).toBe(0)
    expect([rateLabel(0), rateLabel(1), rateLabel(2), rateLabel(-1)]).toEqual([null, null, "2×", "−1×"])
  })

  it("goes back whole frames, carrying the rest", () => {
    // 24 fps backwards at 1×: 10 ms is a quarter frame; four of them make one.
    let carry = 0
    let frames = 0
    for (let i = 0; i < 4; i++) {
      const r = reverseFrames(carry, 1 / 96, 24, -1)
      carry = r.carry
      frames += r.frames
    }
    expect(frames).toBe(1)
    expect(reverseFrames(0, 0.5, 24, -2).frames).toBe(24)
  })
})

describe("the frame store", () => {
  it("draws again only what shows the frame, and what derives from it when that changes", () => {
    const store = new FrameStore()
    const renders = { frame: vi.fn(), shot: vi.fn() }
    const shotOf = (f: number) => Math.floor(f / 48)
    function Timecode() {
      renders.frame()
      return <span>{useFrame(store)}</span>
    }
    function Shot() {
      renders.shot()
      return <span>{useFrameSelector(store, shotOf)}</span>
    }
    const { container } = render(
      <>
        <Timecode />
        <Shot />
      </>,
    )
    const count = () => ({ frame: renders.frame.mock.calls.length, shot: renders.shot.mock.calls.length })
    const before = count()
    for (let f = 1; f <= 50; f++) act(() => store.set(f))
    expect(container.textContent).toBe("501")
    expect(count().frame - before.frame).toBe(50)
    // The shot changed once (47 → 48).
    expect(count().shot - before.shot).toBe(1)
  })
})
