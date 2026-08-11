import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import {
  ChevronFirst,
  ChevronLast,
  Pause,
  Play,
  Repeat,
  StepBack,
  StepForward,
} from "lucide-react"
import { FPS_FALLBACK } from "@/lib/api"
import { formatTimecode, frameAt } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { useVideoClock, type PlayRange } from "./useVideoClock"

export type PlayerSegment = {
  key: string
  startSec: number
  endSec: number
  excluded?: boolean
  /** Visual tone on the scrub bar. */
  tone?: "default" | "manual" | "scene"
}

export type VideoPlayerProps = {
  /** Omit / null while proxy warms — never pass "" (React reloads the page). */
  src?: string | null
  /** Source frame rate. Frame controls hide when absent/zero. */
  fps?: number | null
  durationSec?: number
  segments?: PlayerSegment[]
  /**
   * "source" plays the whole video and treats segments as markers.
   * "reel" confines playback to segments and auto-advances across cuts.
   */
  mode?: "source" | "reel"
  activeKey?: string | null
  onActiveChange?: (key: string | null) => void
  loop?: boolean
  onLoopChange?: (loop: boolean) => void
  /** Bump `nonce` to re-seek even when `sec` is unchanged. */
  seekRequest?: { sec: number; nonce: number } | null
  onTimeUpdate?: (sec: number) => void
  controls?: "full" | "compact" | "none"
  /**
   * Bind transport shortcuts at the document level so they work without
   * focusing the player first. Only for pages with a single primary player.
   */
  globalKeys?: boolean
  muted?: boolean
  autoPlay?: boolean
  className?: string
  stageClassName?: string
  /** Overlay content rendered above the video (hints, WITH chips). */
  children?: ReactNode
}

const SPEEDS = [0.25, 0.5, 1, 1.5, 2]

/**
 * Shared transport for Timeline, Gallery reel and Export hover preview.
 *
 * Replaces three separate <video> implementations that each re-derived
 * playback logic (and each carried their own copy of the timeupdate
 * polling bug). Boundary handling lives in useVideoClock.
 */
export function VideoPlayer({
  src,
  fps,
  durationSec,
  segments = [],
  mode = "source",
  activeKey = null,
  onActiveChange,
  loop = false,
  onLoopChange,
  seekRequest,
  onTimeUpdate,
  controls = "full",
  globalKeys = false,
  muted,
  autoPlay,
  className,
  stageClassName,
  children,
}: VideoPlayerProps) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const containerRef = useRef<HTMLDivElement | null>(null)
  const barRef = useRef<HTMLDivElement | null>(null)
  const [scrubbing, setScrubbing] = useState(false)

  const hasFps = Boolean(fps && fps > 0)
  const effFps = hasFps ? (fps as number) : FPS_FALLBACK

  const ordered = useMemo(
    () =>
      [...segments].sort(
        (a, b) => a.startSec - b.startSec || a.endSec - b.endSec,
      ),
    [segments],
  )
  const activeIndex = useMemo(() => {
    if (!ordered.length) return -1
    const byKey = ordered.findIndex((s) => s.key === activeKey)
    return byKey >= 0 ? byKey : mode === "reel" ? 0 : -1
  }, [ordered, activeKey, mode])
  const activeSegment = activeIndex >= 0 ? ordered[activeIndex] : null

  // Only reel mode confines playback; in source mode the whole video plays
  // and a loop toggle still fences the active segment when one is chosen.
  const activeRange = useMemo<PlayRange | null>(
    () =>
      activeSegment && (mode === "reel" || loop)
        ? { startSec: activeSegment.startSec, endSec: activeSegment.endSec }
        : null,
    [activeSegment, mode, loop],
  )

  const clockRef = useRef<ReturnType<typeof useVideoClock> | null>(null)

  const handleRangeEnd = useCallback(() => {
    const clock = clockRef.current
    if (!clock || !activeSegment) return
    if (loop) {
      clock.seekTo(activeSegment.startSec, { play: clock.playing })
      return
    }
    if (mode === "reel") {
      const next = ordered[activeIndex + 1]
      if (next) {
        onActiveChange?.(next.key)
        clock.seekTo(next.startSec, { play: clock.playing })
      } else {
        clock.pause()
        clock.seekTo(activeSegment.endSec)
      }
      return
    }
    clock.pause()
  }, [activeSegment, activeIndex, ordered, loop, mode, onActiveChange])

  const clock = useVideoClock({
    videoRef,
    fps: effFps,
    activeRange,
    onRangeEnd: handleRangeEnd,
    onTimeUpdate,
  })
  clockRef.current = clock

  const { seekTo, stepFrames, play, pause, setRate } = clock
  const duration = durationSec || clock.duration || 0

  // Controlled seek. Keyed on nonce so re-requesting the same second
  // re-seeks — the old VideoPreview silently ignored that.
  const lastNonce = useRef<number | null>(null)
  useEffect(() => {
    if (!seekRequest) return
    if (lastNonce.current === seekRequest.nonce) return
    lastNonce.current = seekRequest.nonce
    seekTo(seekRequest.sec)
  }, [seekRequest, seekTo])

  // Reel mode: land on the active segment when it changes from outside.
  const lastActiveKey = useRef<string | null>(null)
  useEffect(() => {
    if (mode !== "reel" || !activeSegment) return
    if (lastActiveKey.current === activeSegment.key) return
    lastActiveKey.current = activeSegment.key
    seekTo(activeSegment.startSec)
  }, [mode, activeSegment, seekTo])

  const jumpSegment = useCallback(
    (delta: number) => {
      if (!ordered.length) return
      const from =
        activeIndex >= 0
          ? activeIndex
          : ordered.findIndex((s) => s.startSec > clock.readTime()) - 1
      const next = Math.min(ordered.length - 1, Math.max(0, from + delta))
      const seg = ordered[next]
      if (!seg) return
      onActiveChange?.(seg.key)
      seekTo(seg.startSec, { play: clock.playing })
    },
    [ordered, activeIndex, clock, onActiveChange, seekTo],
  )

  const togglePlay = useCallback(() => {
    if (clock.playing) {
      pause()
      return
    }
    // Restart from the top when parked at the end of the active segment.
    if (activeRange && clock.readTime() >= activeRange.endSec - 1 / effFps) {
      seekTo(activeRange.startSec, { play: true })
      return
    }
    if (activeRange && clock.readTime() < activeRange.startSec) {
      seekTo(activeRange.startSec, { play: true })
      return
    }
    void play()
  }, [clock, activeRange, effFps, pause, play, seekTo])

  // J/K/L shuttle: repeated presses ramp the rate, standard NLE behaviour.
  const shuttleRef = useRef(0)
  const shuttle = useCallback(
    (dir: -1 | 1) => {
      if (dir === 1) {
        shuttleRef.current = shuttleRef.current > 0 ? Math.min(4, shuttleRef.current * 2) : 1
        setRate(shuttleRef.current)
        void play()
      } else {
        // Browsers cannot play backwards; step back a beat per press.
        shuttleRef.current = shuttleRef.current < 0 ? Math.max(-4, shuttleRef.current * 2) : -1
        pause()
        stepFrames(Math.round(shuttleRef.current * effFps * 0.25))
      }
    },
    [setRate, play, pause, stepFrames, effFps],
  )

  /** Returns true when the key was handled, so callers know to preventDefault. */
  const handleKey = useCallback(
    (key: string, shiftKey: boolean): boolean => {
      switch (key) {
        case " ":
          togglePlay()
          return true
        case "ArrowLeft":
          if (shiftKey) seekTo(Math.max(0, clock.readTime() - 1))
          else stepFrames(-1)
          return true
        case "ArrowRight":
          if (shiftKey) seekTo(clock.readTime() + 1)
          else stepFrames(1)
          return true
        case "ArrowUp":
          jumpSegment(-1)
          return true
        case "ArrowDown":
          jumpSegment(1)
          return true
        case "j":
        case "J":
          shuttle(-1)
          return true
        case "k":
        case "K":
          shuttleRef.current = 0
          setRate(1)
          pause()
          return true
        case "l":
        case "L":
          shuttle(1)
          return true
        default:
          return false
      }
    },
    [togglePlay, seekTo, stepFrames, jumpSegment, shuttle, setRate, pause, clock],
  )

  /** True when a keystroke belongs to something else on the page. */
  const shouldIgnoreKeyTarget = (target: EventTarget | null): boolean => {
    const el = target as HTMLElement | null
    if (!el || !el.tagName) return false
    return (
      el.tagName === "INPUT" ||
      el.tagName === "TEXTAREA" ||
      el.tagName === "SELECT" ||
      el.isContentEditable === true
    )
  }

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (globalKeys) return // the document listener owns these
      if (shouldIgnoreKeyTarget(e.target)) return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      if (handleKey(e.key, e.shiftKey)) e.preventDefault()
    },
    [globalKeys, handleKey],
  )

  /**
   * Page-level shortcuts: having to click the player first before space or
   * the arrow keys did anything was needless friction on the Timeline.
   * Still yields to text fields, native menu accelerators (Cmd/Ctrl/Alt)
   * and any open dialog.
   */
  useEffect(() => {
    if (!globalKeys) return
    const onDocKey = (e: KeyboardEvent) => {
      if (shouldIgnoreKeyTarget(e.target)) return
      if (e.metaKey || e.ctrlKey || e.altKey) return
      if (document.querySelector('[role="dialog"][data-state="open"]')) return
      if (handleKey(e.key, e.shiftKey)) e.preventDefault()
    }
    document.addEventListener("keydown", onDocKey)
    return () => document.removeEventListener("keydown", onDocKey)
  }, [globalKeys, handleKey])

  const timeFromClientX = useCallback(
    (clientX: number) => {
      const el = barRef.current
      if (!el || duration <= 0) return 0
      const rect = el.getBoundingClientRect()
      const pct = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width))
      return pct * duration
    },
    [duration],
  )

  const onBarPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (duration <= 0) return
      e.currentTarget.setPointerCapture(e.pointerId)
      setScrubbing(true)
      seekTo(timeFromClientX(e.clientX))
    },
    [duration, seekTo, timeFromClientX],
  )
  const onBarPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!scrubbing) return
      seekTo(timeFromClientX(e.clientX))
    },
    [scrubbing, seekTo, timeFromClientX],
  )
  const endScrub = useCallback((e: React.PointerEvent) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId)
    }
    setScrubbing(false)
  }, [])

  const playheadPct = duration > 0 ? (clock.timeSec / duration) * 100 : 0
  const compact = controls === "compact"

  return (
    <div
      ref={containerRef}
      tabIndex={controls === "none" || globalKeys ? -1 : 0}
      onKeyDown={onKeyDown}
      className={cn(
        "flex min-w-0 flex-col gap-2 outline-none",
        controls !== "none" &&
          !globalKeys &&
          "focus-visible:ring-1 focus-visible:ring-primary/40 focus-visible:ring-offset-2 focus-visible:ring-offset-background rounded-lg",
        className,
      )}
    >
      {/*
        Stage owns the aspect box and the video fills it absolutely: a bare
        <video> ignores CSS height and expands to its intrinsic frame size,
        which blew out the dossier rail.
      */}
      <div
        className={cn(
          "media-above-grain panel-inset relative aspect-video w-full overflow-hidden bg-black",
          stageClassName,
        )}
      >
        <video
          ref={videoRef}
          src={src || undefined}
          muted={muted}
          autoPlay={autoPlay}
          playsInline
          preload="metadata"
          className="absolute inset-0 size-full bg-black object-contain"
          onClick={controls === "none" ? undefined : togglePlay}
        />
        {children}
      </div>

      {controls === "none" ? null : (
        <div className="flex min-w-0 flex-col gap-1.5">
          {/* Scrub bar: same block model as the Timeline so the two agree. */}
          <div
            ref={barRef}
            role="slider"
            aria-label="Scrub"
            aria-valuemin={0}
            aria-valuemax={Math.round(duration)}
            aria-valuenow={Math.round(clock.timeSec)}
            tabIndex={-1}
            onPointerDown={onBarPointerDown}
            onPointerMove={onBarPointerMove}
            onPointerUp={endScrub}
            onPointerCancel={endScrub}
            className="player-scrub"
          >
            {duration > 0 &&
              ordered.map((s) => {
                const left = (s.startSec / duration) * 100
                const width = Math.max(
                  0.4,
                  ((s.endSec - s.startSec) / duration) * 100,
                )
                return (
                  <span
                    key={s.key}
                    className={cn(
                      "player-scrub-mark",
                      s.key === activeKey && "player-scrub-mark-active",
                      s.excluded && "player-scrub-mark-excluded",
                      s.tone === "manual" && "player-scrub-mark-manual",
                    )}
                    style={{ left: `${left}%`, width: `${width}%` }}
                  />
                )
              })}
            <span
              className="player-scrub-head"
              style={{ left: `${Math.min(100, Math.max(0, playheadPct))}%` }}
            />
          </div>

          <div className="flex min-w-0 flex-wrap items-center gap-1">
            <TransportButton label="Previous segment" onClick={() => jumpSegment(-1)} disabled={!ordered.length}>
              <ChevronFirst className="size-3.5" />
            </TransportButton>
            {hasFps ? (
              <TransportButton label="Step back one frame" onClick={() => stepFrames(-1)}>
                <StepBack className="size-3.5" />
              </TransportButton>
            ) : null}
            <TransportButton
              label={clock.playing ? "Pause" : "Play"}
              onClick={togglePlay}
              primary
            >
              {clock.playing ? <Pause className="size-4" /> : <Play className="size-4" />}
            </TransportButton>
            {hasFps ? (
              <TransportButton label="Step forward one frame" onClick={() => stepFrames(1)}>
                <StepForward className="size-3.5" />
              </TransportButton>
            ) : null}
            <TransportButton label="Next segment" onClick={() => jumpSegment(1)} disabled={!ordered.length}>
              <ChevronLast className="size-3.5" />
            </TransportButton>

            <TransportButton
              label={loop ? "Stop looping segment" : "Loop current segment"}
              onClick={() => onLoopChange?.(!loop)}
              disabled={!onLoopChange || !activeSegment}
              active={loop}
            >
              <Repeat className="size-3.5" />
            </TransportButton>

            <span className="timecode ml-1 text-[11px] tabular-nums text-muted-foreground">
              {formatTimecode(clock.timeSec, hasFps ? effFps : 0)}
              {hasFps && !compact ? (
                <span className="ml-1.5 opacity-60">f{frameAt(clock.timeSec, effFps)}</span>
              ) : null}
            </span>

            <select
              aria-label="Playback speed"
              value={clock.rate}
              onChange={(e) => setRate(Number(e.target.value))}
              className="ml-auto rounded-md border border-border/50 bg-background/60 px-1.5 py-1 text-[11px] tabular-nums text-muted-foreground outline-none transition hover:border-border focus-visible:border-primary/50"
            >
              {SPEEDS.map((s) => (
                <option key={s} value={s}>
                  {s}×
                </option>
              ))}
            </select>
          </div>
        </div>
      )}
    </div>
  )
}

function TransportButton({
  label,
  onClick,
  disabled,
  primary,
  active,
  children,
}: {
  label: string
  onClick: () => void
  disabled?: boolean
  primary?: boolean
  active?: boolean
  children: ReactNode
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "inline-flex items-center justify-center rounded-md border transition disabled:pointer-events-none disabled:opacity-35",
        primary ? "size-8" : "size-7",
        active
          ? "border-primary/50 bg-primary/15 text-primary"
          : "border-border/45 bg-card/40 text-muted-foreground hover:border-border hover:text-foreground",
      )}
    >
      {children}
    </button>
  )
}
