import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import type { FrameClock } from "@/lib/frame-clock"
import { FrameStore } from "./frame-store"
import { type ShuttleKey, nextRate, reverseFrames } from "./shuttle"

type VideoFrameMeta = { mediaTime: number }
type RvfcVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: (now: number, meta: VideoFrameMeta) => void) => number
  cancelVideoFrameCallback?: (handle: number) => void
}

/**
 * Frame-accurate control of a `<video>` element.
 *
 * The displayed frame comes from `requestVideoFrameCallback` (the timestamp of
 * the frame actually composited), never from `currentTime`, which runs ahead
 * of the picture during playback. Seeks target the middle of a frame. The
 * frame lives in a `FrameStore`, not in React state: only what shows it draws
 * again on every video frame.
 *
 * `rate` is the shuttle's: 0 stopped, 1 playing, 2 or 4 fast, below 0
 * backwards — played as a run of seeks, smooth on intra-frame previews and
 * slower on long-GOP files played directly.
 */
export function usePlayer(clock: FrameClock | null) {
  const videoRef = useRef<RvfcVideo | null>(null)
  const store = useMemo(() => new FrameStore(), [])
  const [rate, setRate] = useState(0)
  const [ready, setReady] = useState(false)
  const target = useRef<number | null>(null)
  const backwards = useRef<{ frame: number; carry: number } | null>(null)

  // Track the presented frame.
  useEffect(() => {
    const v = videoRef.current
    if (!v || !clock) return
    let handle = 0
    let raf = 0
    if (v.requestVideoFrameCallback) {
      const onFrame = (_: number, meta: VideoFrameMeta) => {
        const f = clock.frameAt(meta.mediaTime)
        // While a seek is pending, ignore frames from before it.
        if (target.current == null || target.current === f) {
          target.current = null
          store.set(f)
        }
        handle = v.requestVideoFrameCallback!(onFrame)
      }
      handle = v.requestVideoFrameCallback(onFrame)
      return () => v.cancelVideoFrameCallback?.(handle)
    }
    // Fallback (no rVFC): sample currentTime once per animation frame.
    const tick = () => {
      if (target.current == null) store.set(clock.frameAt(v.currentTime))
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [clock, store])

  const go = useCallback(
    (f: number) => {
      const v = videoRef.current
      if (!v || !clock) return
      const c = clock.clamp(f)
      target.current = c
      store.set(c)
      v.currentTime = clock.seekTime(c)
    },
    [clock, store],
  )

  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    const onPlay = () => setRate((r) => (r > 0 ? r : 1))
    const onPause = () => {
      // Playing backwards is a run of seeks on a paused video.
      if (backwards.current) return
      setRate(0)
      // rVFC can lag behind (throttled tabs) and doesn't fire on pause: snap
      // to the frame actually on screen so stepping continues from there.
      if (!clock || target.current != null) return
      const f = clock.frameAt(v.currentTime)
      target.current = f
      store.set(f)
      v.currentTime = clock.seekTime(f)
    }
    const onReady = () => setReady(true)
    const onSeeked = () => {
      // rVFC may not fire for a seek that lands on the frame already shown.
      if (target.current != null && clock) {
        store.set(target.current)
        target.current = null
      }
    }
    v.addEventListener("play", onPlay)
    v.addEventListener("pause", onPause)
    v.addEventListener("ended", onPause)
    v.addEventListener("loadeddata", onReady)
    v.addEventListener("seeked", onSeeked)
    if (v.readyState >= 2) setReady(true)
    return () => {
      v.removeEventListener("play", onPlay)
      v.removeEventListener("pause", onPause)
      v.removeEventListener("ended", onPause)
      v.removeEventListener("loadeddata", onReady)
      v.removeEventListener("seeked", onSeeked)
    }
  }, [clock, store])

  // Backwards: a frame (or more, faster) back as often as the rate asks,
  // each once the previous one is on screen.
  useEffect(() => {
    if (rate >= 0 || !clock) return
    const v = videoRef.current
    if (!v) return
    const fps = 1 / Math.max(1e-6, clock.duration(store.get()))
    backwards.current = { frame: store.get(), carry: 0 }
    let last = performance.now()
    let raf = 0
    const tick = (now: number) => {
      const b = backwards.current
      if (!b) return
      const r = reverseFrames(b.carry, (now - last) / 1000, fps, rate)
      last = now
      b.carry = r.carry
      if (r.frames > 0 && target.current == null && !v.seeking) {
        b.frame = Math.max(0, store.get() - r.frames)
        go(b.frame)
        if (b.frame === 0) {
          backwards.current = null
          setRate(0)
          return
        }
      }
      raf = requestAnimationFrame(tick)
    }
    v.pause()
    raf = requestAnimationFrame(tick)
    return () => {
      cancelAnimationFrame(raf)
      backwards.current = null
    }
  }, [rate, clock, store, go])

  const stop = useCallback(() => {
    backwards.current = null
    setRate(0)
    const v = videoRef.current
    if (v) {
      v.playbackRate = 1
      v.pause()
    }
  }, [])

  const seek = useCallback(
    (f: number) => {
      if (backwards.current) stop()
      go(f)
    },
    [go, stop],
  )

  const pause = stop

  const play = useCallback(
    (at: number) => {
      const v = videoRef.current
      if (!v || !clock) return
      backwards.current = null
      // Restart from the top when parked on the last frame.
      if (store.get() >= clock.frames - 1) go(0)
      v.playbackRate = at
      setRate(at)
      void v.play()
    },
    [clock, store, go],
  )

  const toggle = useCallback(() => {
    if (rate !== 0) stop()
    else play(1)
  }, [rate, stop, play])

  const step = useCallback(
    (delta: number) => {
      stop()
      go((target.current ?? store.get()) + delta)
    },
    [stop, go, store],
  )

  /** J, K or L pressed. */
  const shuttle = useCallback(
    (key: ShuttleKey) => {
      const next = nextRate(rate, key)
      if (next === 0) stop()
      else if (next > 0) play(next)
      else {
        videoRef.current?.pause()
        setRate(next)
      }
    },
    [rate, stop, play],
  )

  return { videoRef, store, rate, playing: rate !== 0, ready, seek, step, toggle, pause, shuttle }
}
