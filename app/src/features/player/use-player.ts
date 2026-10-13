import { useCallback, useEffect, useRef, useState } from "react"
import type { FrameClock } from "@/lib/frame-clock"

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
 * of the picture during playback. Seeks target the middle of a frame.
 */
export function usePlayer(clock: FrameClock | null) {
  const videoRef = useRef<RvfcVideo | null>(null)
  const [frame, setFrame] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [ready, setReady] = useState(false)
  const target = useRef<number | null>(null)

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
          setFrame(f)
        }
        handle = v.requestVideoFrameCallback!(onFrame)
      }
      handle = v.requestVideoFrameCallback(onFrame)
      return () => v.cancelVideoFrameCallback?.(handle)
    }
    // Fallback (no rVFC): sample currentTime once per animation frame.
    const tick = () => {
      if (target.current == null) setFrame(clock.frameAt(v.currentTime))
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [clock])

  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    const onPlay = () => setPlaying(true)
    const onPause = () => {
      setPlaying(false)
      // rVFC can lag behind (throttled tabs) and doesn't fire on pause: snap
      // to the frame actually on screen so stepping continues from there.
      if (!clock || target.current != null) return
      const f = clock.frameAt(v.currentTime)
      target.current = f
      setFrame(f)
      v.currentTime = clock.seekTime(f)
    }
    const onReady = () => setReady(true)
    const onSeeked = () => {
      // rVFC may not fire for a seek that lands on the frame already shown.
      if (target.current != null && clock) {
        setFrame(target.current)
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
  }, [clock])

  const seek = useCallback(
    (f: number) => {
      const v = videoRef.current
      if (!v || !clock) return
      const c = clock.clamp(f)
      target.current = c
      setFrame(c)
      v.currentTime = clock.seekTime(c)
    },
    [clock],
  )

  const pause = useCallback(() => videoRef.current?.pause(), [])

  const toggle = useCallback(() => {
    const v = videoRef.current
    if (!v || !clock) return
    if (!v.paused) {
      v.pause()
      return
    }
    // Restart from the top when parked on the last frame.
    if (frame >= clock.frames - 1) seek(0)
    void v.play()
  }, [clock, frame, seek])

  const step = useCallback(
    (delta: number) => {
      videoRef.current?.pause()
      seek((target.current ?? frame) + delta)
    },
    [frame, seek],
  )

  return { videoRef, frame, playing, ready, seek, step, toggle, pause }
}
