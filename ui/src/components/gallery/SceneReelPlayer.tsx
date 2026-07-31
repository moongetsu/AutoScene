import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { AnimatePresence, motion } from "motion/react"
import { Pause, Play, SkipBack, SkipForward } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Slider } from "@/components/ui/slider"
import { api, appendQuery, formatDuration, type Person, type Segment } from "@/lib/api"
import { coCastForSegment } from "@/lib/scenes"
import { friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type SceneReelPlayerProps = {
  projectId: string
  segments: Segment[]
  /** Full project segments — honest co-cast needs everyone's face windows. */
  allSegments?: Segment[]
  people?: Person[]
  onSelectPerson?: (personId: number) => void
  className?: string
}

function sceneLen(s: Segment) {
  return Math.max(0.05, s.end_sec - s.start_sec)
}

function segmentsFingerprint(segments: Segment[]) {
  return segments
    .map(
      (s) =>
        `${s.person_id}:${s.start_sec.toFixed(3)}:${s.end_sec.toFixed(3)}`,
    )
    .join("|")
}

export function SceneReelPlayer({
  projectId,
  segments,
  allSegments,
  people = [],
  onSelectPerson,
  className,
}: SceneReelPlayerProps) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  /** Ignore pause events fired by seeks while we are intentionally playing. */
  const playIntentRef = useRef(false)
  const segKey = useMemo(() => segmentsFingerprint(segments), [segments])
  const segmentsRef = useRef(segments)
  segmentsRef.current = segments
  const ordered = useMemo(() => {
    return [...segmentsRef.current].sort(
      (a, b) => a.start_sec - b.start_sec || a.end_sec - b.end_sec,
    )
  }, [segKey])
  const [sceneIdx, setSceneIdx] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [ready, setReady] = useState(false)
  const [localTime, setLocalTime] = useState(0)
  const [cutFlash, setCutFlash] = useState(0)
  const [loadHint, setLoadHint] = useState<string | null>(null)
  /** Cache-bust only on project change — remux URL stays stable across segment polls. */
  const videoSrc = useMemo(
    () => appendQuery(api.videoUrl(projectId), { v: projectId }),
    [projectId],
  )
  const scene = ordered[sceneIdx] ?? null

  useEffect(() => {
    let cancelled = false
    setLoadHint("Loading video…")
    void (async () => {
      try {
        const st = await api.videoProxyStatus(projectId)
        if (cancelled) return
        if (!st.native_ok && !st.proxy_ready) {
          setLoadHint(
            st.needs_transcode
              ? "Preparing video preview (one-time)…"
              : "Preparing video preview…",
          )
        } else {
          setLoadHint(null)
        }
      } catch {
        if (!cancelled) setLoadHint(null)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [projectId])
  // "Also in scene" only when another face window truly overlaps this clip —
  // never from time-union scenes (those glued whole dialogue exchanges).
  const coCastPool = allSegments?.length ? allSegments : segments
  const others = useMemo(
    () => (scene ? coCastForSegment(scene, coCastPool, people) : []),
    [scene, coCastPool, people],
  )

  const playVideo = useCallback(async () => {
    const v = videoRef.current
    if (!v) return false
    playIntentRef.current = true
    try {
      await v.play()
      setPlaying(true)
      return true
    } catch (err) {
      playIntentRef.current = false
      setPlaying(false)
      const msg = friendlyError(err, "Playback failed")
      toast.error(msg)
      return false
    }
  }, [])

  const seekToScene = useCallback(
    (index: number, autoplay: boolean) => {
      const v = videoRef.current
      const s = ordered[index]
      if (!v || !s) return
      setSceneIdx(index)
      setCutFlash((n) => n + 1)
      const t = s.start_sec + 0.001
      const apply = () => {
        try {
          v.currentTime = t
        } catch {
          /* seek may fail before metadata */
        }
        setLocalTime(0)
        if (autoplay) void playVideo()
      }
      if (v.readyState >= 1) apply()
      else v.addEventListener("loadedmetadata", apply, { once: true })
    },
    [ordered, playVideo],
  )

  // Reset only when project or segment *content* changes — not every 1.2s poll
  // that rebuilds a new segments[] array with the same cuts.
  useEffect(() => {
    setSceneIdx(0)
    setPlaying(false)
    playIntentRef.current = false
    setLocalTime(0)
    setReady(false)
    const v = videoRef.current
    if (v) {
      v.pause()
      if (ordered[0]) {
        try {
          v.currentTime = ordered[0].start_sec
        } catch {
          /* ignore */
        }
      }
    }
  }, [projectId, segKey])

  const onTimeUpdate = () => {
    const v = videoRef.current
    if (!v || !scene) return
    const rel = Math.max(0, v.currentTime - scene.start_sec)
    setLocalTime(rel)
    if (v.currentTime >= scene.end_sec - 0.04) {
      if (sceneIdx < ordered.length - 1) {
        seekToScene(sceneIdx + 1, playIntentRef.current || playing)
      } else {
        playIntentRef.current = false
        v.pause()
        setPlaying(false)
        setLocalTime(sceneLen(scene))
      }
    }
  }

  const togglePlay = async () => {
    const v = videoRef.current
    if (!v || !scene) return
    if (playing || !v.paused) {
      playIntentRef.current = false
      v.pause()
      setPlaying(false)
      return
    }
    const outOfScene =
      Number.isNaN(v.currentTime) ||
      v.currentTime < scene.start_sec - 0.02 ||
      v.currentTime >= scene.end_sec - 0.05
    if (outOfScene) {
      seekToScene(sceneIdx, true)
      return
    }
    await playVideo()
  }

  const scrubScene = (rel: number) => {
    const v = videoRef.current
    if (!v || !scene) return
    const clamped = Math.min(sceneLen(scene), Math.max(0, rel))
    v.currentTime = scene.start_sec + clamped
    setLocalTime(clamped)
  }

  const nudge = (delta: number) => {
    const v = videoRef.current
    if (!v || !scene) return
    const next = Math.min(
      scene.end_sec - 0.05,
      Math.max(scene.start_sec, v.currentTime + delta),
    )
    v.currentTime = next
    setLocalTime(next - scene.start_sec)
  }

  if (!ordered.length) {
    return (
      <div
        className={cn(
          "rounded-xl border border-border/40 bg-background/40 px-4 py-6 text-center text-sm text-muted-foreground",
          className,
        )}
      >
        No scenes for this character yet.
      </div>
    )
  }

  const scenePct = scene ? (localTime / sceneLen(scene)) * 100 : 0

  return (
    <div className={cn("min-w-0 max-w-full space-y-3 overflow-hidden", className)}>
      {/*
        Cap the stage: bare <video aspect-video> often ignores CSS and expands
        to intrinsic frame height (huge in the dossier rail).
      */}
      <div className="media-above-grain mx-auto aspect-video w-full max-h-[200px] max-w-full overflow-hidden rounded-lg border border-border/40 bg-[oklch(0.06_0.01_280)]">
        <AnimatePresence mode="sync">
          <motion.div
            key={cutFlash}
            initial={{ opacity: 0.35 }}
            animate={{ opacity: 0 }}
            transition={{ duration: 0.35 }}
            className="pointer-events-none absolute inset-0 z-10 bg-primary/20"
          />
        </AnimatePresence>
        <video
          ref={videoRef}
          src={videoSrc}
          className="absolute inset-0 size-full object-contain"
          preload="metadata"
          playsInline
          onLoadStart={() => {
            if (!ready) setLoadHint((h) => h ?? "Loading video…")
          }}
          onLoadedMetadata={() => {
            setReady(true)
            setLoadHint(null)
            if (scene) {
              try {
                videoRef.current!.currentTime = scene.start_sec
              } catch {
                /* ignore */
              }
            }
          }}
          onCanPlay={() => setLoadHint(null)}
          onTimeUpdate={onTimeUpdate}
          onPause={() => {
            // Seeks often emit pause; keep UI in play if user asked to play.
            if (playIntentRef.current) return
            setPlaying(false)
          }}
          onPlay={() => {
            playIntentRef.current = true
            setPlaying(true)
          }}
          onError={() => {
            playIntentRef.current = false
            setPlaying(false)
            setReady(false)
            const v = videoRef.current
            const code = v?.error?.code
            // MEDIA_ERR_SRC_NOT_SUPPORTED === 4
            const detail =
              code === 4
                ? "No supported video format. Try re-importing or check your source file."
                : "Could not load project video for the reel"
            setLoadHint(detail)
            toast.error(friendlyError(detail))
          }}
        />
        {loadHint && !ready ? (
          <div className="absolute inset-0 z-[5] flex items-center justify-center bg-background/55 px-4 text-center">
            <p className="max-w-[16rem] text-[11px] leading-snug text-muted-foreground">
              {loadHint}
            </p>
          </div>
        ) : null}
        <div className="absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 bg-gradient-to-t from-background/95 via-background/35 to-transparent px-2.5 pb-2 pt-6">
          <p className="timecode text-[10px] tabular-nums text-primary">
            {String(sceneIdx + 1).padStart(2, "0")}/{String(ordered.length).padStart(2, "0")}
          </p>
          <p className="timecode text-[10px] tabular-nums text-muted-foreground">
            {scene
              ? `${formatDuration(scene.start_sec)} - ${formatDuration(scene.end_sec)}`
              : "-"}
          </p>
        </div>
      </div>

      {others.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-[9px] uppercase tracking-[0.16em] text-muted-foreground">
            Also in scene
          </span>
          {others.map((p) => (
            <button
              key={p.person_id}
              type="button"
              onClick={() => onSelectPerson?.(p.person_id)}
              className="inline-flex items-center gap-1.5 rounded-md border border-border/40 bg-card/50 px-1.5 py-1 text-left text-[11px] transition hover:border-primary/40 hover:bg-primary/5"
            >
              {p.thumbnail_path ? (
                <img
                  src={api.mediaUrl(projectId, p.thumbnail_path)}
                  alt=""
                  className="size-4 rounded-sm object-cover"
                />
              ) : null}
              <span className="max-w-[7rem] truncate">{p.label}</span>
            </button>
          ))}
        </div>
      ) : null}

      <div className="space-y-1.5">
        <div className="flex items-center justify-between text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
          <span>In-scene</span>
          <span className="timecode tabular-nums text-foreground/80">
            {formatDuration(localTime)} / {scene ? formatDuration(sceneLen(scene)) : "-"}
          </span>
        </div>
        <Slider
          min={0}
          max={1000}
          step={1}
          value={[Math.round(Math.min(1000, Math.max(0, scenePct * 10)))]}
          onValueChange={(v) => {
            const n = Array.isArray(v) ? v[0] : v
            if (typeof n !== "number" || !scene) return
            scrubScene((n / 1000) * sceneLen(scene))
          }}
          disabled={!ready || !scene}
        />
      </div>

      <div className="flex flex-wrap items-center justify-center gap-1.5">
        <Button
          type="button"
          size="icon-sm"
          variant="outline"
          className="size-9"
          disabled={sceneIdx <= 0}
          onClick={() => seekToScene(sceneIdx - 1, playing)}
          aria-label="Previous scene"
        >
          <SkipBack className="size-3.5" />
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-9 min-w-11 px-2 timecode text-[11px] tabular-nums"
          onClick={() => nudge(-1)}
          aria-label="Back one second"
        >
          -1s
        </Button>
        <Button
          type="button"
          size="icon"
          className="size-11 rounded-full"
          onClick={() => void togglePlay()}
          aria-label={playing ? "Pause reel" : "Play reel"}
        >
          {playing ? <Pause className="size-4" /> : <Play className="size-4 pl-0.5" />}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-9 min-w-11 px-2 timecode text-[11px] tabular-nums"
          onClick={() => nudge(1)}
          aria-label="Forward one second"
        >
          +1s
        </Button>
        <Button
          type="button"
          size="icon-sm"
          variant="outline"
          className="size-9"
          disabled={sceneIdx >= ordered.length - 1}
          onClick={() => seekToScene(sceneIdx + 1, playing)}
          aria-label="Next scene"
        >
          <SkipForward className="size-3.5" />
        </Button>
      </div>

      <div className="filmstrip-fade min-w-0 max-w-full">
        <div className="filmstrip-scroll flex max-w-full gap-1.5 pb-0.5">
          {ordered.map((s, i) => {
            const active = i === sceneIdx
            const castCount = 1 + coCastForSegment(s, coCastPool, people).length
            return (
              <button
                key={`${s.start_sec}-${s.end_sec}-${i}`}
                type="button"
                onClick={() => seekToScene(i, playing)}
                className={cn(
                  "shrink-0 rounded-md border px-2 py-1.5 text-left transition",
                  active
                    ? "border-primary/50 bg-primary/10"
                    : "border-border/35 bg-card/40 hover:border-border/70",
                )}
              >
                <p
                  className={cn(
                    "timecode text-[10px] tabular-nums",
                    active ? "text-primary" : "text-muted-foreground",
                  )}
                >
                  {String(i + 1).padStart(2, "0")}
                </p>
                <p className="timecode mt-0.5 text-[10px] tabular-nums text-foreground/80">
                  {formatDuration(s.start_sec)}
                </p>
                <p className="timecode text-[9px] tabular-nums text-muted-foreground/70">
                  {formatDuration(sceneLen(s))}
                  {castCount > 1 ? ` · ${castCount}` : ""}
                </p>
              </button>
            )
          })}
        </div>
      </div>
    </div>
  )
}
