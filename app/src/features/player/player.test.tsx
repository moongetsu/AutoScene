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
    // 24 fps backwards at 1×: 1/96 s is a quarter frame; four of them make one.
    let carry = 0
    let frames = 0
    for (let i = 0; i < 4; i++) {
      const r = reverseFrames(carry, 1 / 96, 24, -1)
      carry = r.carry
      frames += r.frames
    }
    expect(frames).toBe(1)
    expect(reverseFrames(0, 0.5, 24, -2).frames).toBe(24)
    // After a stall (a hidden window) no more than a second's worth at once.
    expect(reverseFrames(0, 30, 24, -2).frames).toBe(48)
  })

  it("keeps the speed it shows while seeks take several ticks", () => {
    // 60 Hz ticks, 24 fps; each seek lands `ticks` ticks after it was asked.
    const run = (rate: number, ticks: number) => {
      let carry = 0
      let back = 0
      let busy = 0
      for (let i = 0; i < 120; i++) {
        if (busy > 0) busy--
        const r = reverseFrames(carry, 1 / 60, 24, rate, busy === 0)
        carry = r.carry
        if (r.frames > 0) {
          back += r.frames
          busy = ticks
        }
      }
      return back / 48 // frames in two seconds at 1×
    }
    // Within 5 % (what is still owed when the two seconds end).
    for (const ticks of [1, 2, 3]) {
      for (const rate of [-1, -2, -4]) {
        const speed = run(rate, ticks) / -rate
        expect(speed).toBeGreaterThan(0.95)
        expect(speed).toBeLessThanOrEqual(1)
      }
    }
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
