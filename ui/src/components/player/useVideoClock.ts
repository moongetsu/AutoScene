import { useCallback, useEffect, useRef, useState } from "react"

export type PlayRange = { startSec: number; endSec: number }

type UseVideoClockArgs = {
  videoRef: React.RefObject<HTMLVideoElement | null>
  /** Source frame rate. Drives frame stepping and boundary precision. */
  fps: number
  /** Range playback is confined to. null = play the whole source. */
  activeRange?: PlayRange | null
  /** Fires on the first presented frame at/after activeRange.endSec. */
  onRangeEnd?: () => void
  onTimeUpdate?: (sec: number) => void
  /**
   * Re-attach listeners when this changes. Needed by callers that mount and
   * unmount the <video> element itself (the Export hover tile), where the
   * ref identity alone never tells the effect a new element appeared.
   */
  attachToken?: unknown
}

export type VideoClock = {
  /** Throttled for render. Use readTime() when you need the exact value. */
  timeSec: number
  playing: boolean
  ready: boolean
  duration: number
  rate: number
  readTime: () => number
  seekTo: (sec: number, opts?: { play?: boolean }) => void
  stepFrames: (n: number) => void
  play: () => Promise<boolean>
  pause: () => void
  setRate: (rate: number) => void
}

/** Render at ~30Hz; the clock itself stays frame-exact in a ref. */
const PUBLISH_STEP = 1 / 30

/**
 * Frame-resolution playback clock.
 *
 * The old reel player watched the `timeupdate` event (~4 fires/sec) for
 * `currentTime >= end_sec - 0.04`, so playback routinely ran a quarter
 * second past the cut before jumping — the hitch that the cut-flash
 * overlay existed to hide. requestVideoFrameCallback fires once per
 * presented frame, so a boundary is caught within a single frame and the
 * seek lands before the next one paints. rAF is the fallback where rVFC
 * is unavailable (some WebKit builds).
 */
export function useVideoClock({
  videoRef,
  fps,
  activeRange,
  onRangeEnd,
  onTimeUpdate,
  attachToken,
}: UseVideoClockArgs): VideoClock {
  const [timeSec, setTimeSec] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [ready, setReady] = useState(false)
  const [duration, setDuration] = useState(0)
  const [rate, setRateState] = useState(1)

  const timeRef = useRef(0)
  const publishedRef = useRef(-1)
  /** Suppresses the boundary check between issuing a seek and it landing. */
  const seekingRef = useRef(false)
  const rangeRef = useRef(activeRange ?? null)
  rangeRef.current = activeRange ?? null
  const onRangeEndRef = useRef(onRangeEnd)
  onRangeEndRef.current = onRangeEnd
  const onTimeUpdateRef = useRef(onTimeUpdate)
  onTimeUpdateRef.current = onTimeUpdate

  const frameDur = fps > 0 ? 1 / fps : 1 / 24

  const publish = useCallback(
    (t: number, force = false) => {
      timeRef.current = t
      if (force || Math.abs(t - publishedRef.current) >= PUBLISH_STEP) {
        publishedRef.current = t
        setTimeSec(t)
        onTimeUpdateRef.current?.(t)
      }
    },
    [],
  )

  const readTime = useCallback(() => {
    const v = videoRef.current
    return v ? v.currentTime : timeRef.current
  }, [videoRef])

  const seekTo = useCallback(
    (sec: number, opts?: { play?: boolean }) => {
      const v = videoRef.current
      if (!v) return
      const target = Math.max(0, sec)
      seekingRef.current = true
      const apply = () => {
        try {
          v.currentTime = target
        } catch {
          /* seeking before metadata throws on some engines */
        }
        publish(target, true)
        if (opts?.play) void v.play().catch(() => {})
      }
      // Unlike the old prop-driven seek there is no dead-zone here: asking
      // for the same time twice must re-seek (re-clicking a segment).
      if (v.readyState >= 1) apply()
      else v.addEventListener("loadedmetadata", apply, { once: true })
    },
    [videoRef, publish],
  )

  const stepFrames = useCallback(
    (n: number) => {
      const v = videoRef.current
      if (!v) return
      v.pause()
      // Land mid-frame so rounding never sticks on a boundary.
      const cur = v.currentTime
      const frame = Math.round(cur / frameDur)
      const next = (frame + n) * frameDur + frameDur / 2
      const r = rangeRef.current
      const lo = r ? r.startSec : 0
      const hi = r ? r.endSec - frameDur / 2 : (v.duration || Infinity)
      seekTo(Math.min(Math.max(next, lo), hi))
    },
    [videoRef, frameDur, seekTo],
  )

  const play = useCallback(async () => {
    const v = videoRef.current
    if (!v) return false
    try {
      await v.play()
      return true
    } catch {
      return false
    }
  }, [videoRef])

  const pause = useCallback(() => {
    videoRef.current?.pause()
  }, [videoRef])

  const setRate = useCallback(
    (r: number) => {
      setRateState(r)
      const v = videoRef.current
      if (v) v.playbackRate = r
    },
    [videoRef],
  )

  // Frame-driven clock + boundary detection.
  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    let handle = 0
    let rafHandle = 0
    let cancelled = false
    const useRvfc = typeof v.requestVideoFrameCallback === "function"

    const tick = (mediaTime: number) => {
      if (cancelled) return
      publish(mediaTime)
      const r = rangeRef.current
      if (r && !seekingRef.current && mediaTime >= r.endSec - frameDur / 2) {
        onRangeEndRef.current?.()
      }
    }

    const rvfcLoop = (_now: number, meta: VideoFrameCallbackMetadata) => {
      // mediaTime is the presentation timestamp of the frame actually on
      // screen — more precise than reading currentTime after the fact.
      tick(meta.mediaTime)
      if (!cancelled) handle = v.requestVideoFrameCallback!(rvfcLoop)
    }

    const rafLoop = () => {
      if (cancelled) return
      tick(v.currentTime)
      rafHandle = requestAnimationFrame(rafLoop)
    }

    const start = () => {
      if (useRvfc) handle = v.requestVideoFrameCallback!(rvfcLoop)
      else rafHandle = requestAnimationFrame(rafLoop)
    }
    const stop = () => {
      if (useRvfc && handle) v.cancelVideoFrameCallback?.(handle)
      if (rafHandle) cancelAnimationFrame(rafHandle)
      handle = 0
      rafHandle = 0
    }

    const onPlay = () => {
      setPlaying(true)
      stop()
      start()
    }
    const onPause = () => {
      setPlaying(false)
      stop()
      publish(v.currentTime, true)
    }
    const onSeeked = () => {
      seekingRef.current = false
      publish(v.currentTime, true)
    }
    const onLoaded = () => {
      setReady(true)
      setDuration(v.duration || 0)
      publish(v.currentTime, true)
    }

    v.addEventListener("play", onPlay)
    v.addEventListener("playing", onPlay)
    v.addEventListener("pause", onPause)
    v.addEventListener("seeked", onSeeked)
    v.addEventListener("loadedmetadata", onLoaded)
    if (v.readyState >= 1) onLoaded()
    if (!v.paused) onPlay()

    return () => {
      cancelled = true
      stop()
      v.removeEventListener("play", onPlay)
      v.removeEventListener("playing", onPlay)
      v.removeEventListener("pause", onPause)
      v.removeEventListener("seeked", onSeeked)
      v.removeEventListener("loadedmetadata", onLoaded)
    }
  }, [videoRef, frameDur, publish, attachToken])

  // Keep the element's rate in sync if it remounts.
  useEffect(() => {
    const v = videoRef.current
    if (v) v.playbackRate = rate
  }, [videoRef, rate, ready])

  return {
    timeSec,
    playing,
    ready,
    duration,
    rate,
    readTime,
    seekTo,
    stepFrames,
    play,
    pause,
    setRate,
  }
}
