import { act, cleanup, render, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { FrameClock } from "@/lib/frame-clock"
import { FrameStore, useFrame, useFrameSelector } from "./frame-store"
import { nextRate, rateLabel, reverseFrames } from "./shuttle"
import { usePlayer } from "./use-player"

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

/**
 * A `<video>` as far as the player uses it while paused: a seek is on its
 * way from setting `currentTime` until `land`, and frames are shown when the
 * test says (`show`, as `requestVideoFrameCallback` reports them).
 */
class FakeVideo extends EventTarget {
  seeking = false
  paused = true
  readyState = 4
  playbackRate = 1
  private time = 0
  private callbacks: ((now: number, meta: { mediaTime: number }) => void)[] = []
  get currentTime() {
    return this.time
  }
  set currentTime(t: number) {
    this.time = t
    this.seeking = true
  }
  requestVideoFrameCallback(cb: (now: number, meta: { mediaTime: number }) => void) {
    this.callbacks.push(cb)
    return this.callbacks.length
  }
  cancelVideoFrameCallback() {}
  pause() {}
  play() {
    return Promise.resolve()
  }
  show(mediaTime: number) {
    const due = this.callbacks
    this.callbacks = []
    for (const cb of due) cb(0, { mediaTime })
  }
  /** The seek ends: `seeking` drops at once, `seeked` comes a task later (`report`). */
  land() {
    this.seeking = false
  }
  report() {
    this.dispatchEvent(new Event("seeked"))
  }
}

describe("usePlayer", () => {
  const clock = new FrameClock({ frames: 100, fps: { num: 24, den: 1 }, offset: 0, step: 1 / 24, times: null })

  /** The player on a fake video (attached before the clock arrives, as a `<video>` would be). */
  function player(on = clock) {
    const video = new FakeVideo()
    const hook = renderHook(({ c }) => usePlayer(c), { initialProps: { c: null as FrameClock | null } })
    hook.result.current.videoRef.current = video as unknown as HTMLVideoElement
    hook.rerender({ c: on })
    return { video, hook }
  }

  it("keeps waiting for a seek when an older one reports it has ended", () => {
    const { video, hook } = player()
    const { store } = hook.result.current
    act(() => hook.result.current.seek(10))
    video.land()
    // The next step goes out before the first seek's `seeked` is delivered.
    act(() => hook.result.current.seek(20))
    video.report()
    // The first seek's frame is shown; the playhead stays where it was sent.
    video.show(clock.start(10))
    expect(store.get()).toBe(20)
    // And a step from there starts at 20.
    act(() => hook.result.current.step(1))
    expect(store.get()).toBe(21)
    video.land()
    video.report()
    video.show(clock.start(21))
    expect(store.get()).toBe(21)
  })

  it("lets frames through again once the last seek has ended", () => {
    const { video, hook } = player()
    const { store } = hook.result.current
    act(() => hook.result.current.seek(20))
    // Landing on the frame already shown: no new frame, only `seeked`.
    video.land()
    video.report()
    expect(store.get()).toBe(20)
    video.show(clock.start(30))
    expect(store.get()).toBe(30)
  })

  it("plays backwards at the media's rate from a frame that lasts long", () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance"] })
    try {
      // 25 fps; frame 2 stays two seconds (a gap in a recording).
      const vfr = new FrameClock({
        frames: 5,
        fps: { num: 25, den: 1 },
        offset: 0,
        step: null,
        times: [0, 0.04, 0.08, 2.08, 2.12, 2.16],
      })
      const { video, hook } = player(vfr)
      const { store } = hook.result.current
      act(() => hook.result.current.seek(2))
      video.land()
      video.report()
      act(() => hook.result.current.shuttle("j"))
      // A frame back after a 25th of a second, not after two seconds.
      act(() => vi.advanceTimersByTime(100))
      expect(store.get()).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})
