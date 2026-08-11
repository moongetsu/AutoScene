import { useEffect, useMemo, useState } from "react"
import { api, appendQuery, formatDuration, type Person, type Segment } from "@/lib/api"
import { buildCoCastMap, coCastForSegment, segmentKey } from "@/lib/scenes"
import { VideoPlayer, type PlayerSegment } from "@/components/player/VideoPlayer"
import { useVideoFps } from "@/components/player/useVideoFps"
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
    .map((s) => `${s.person_id}:${s.start_sec.toFixed(3)}:${s.end_sec.toFixed(3)}`)
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
  const segKey = useMemo(() => segmentsFingerprint(segments), [segments])
  const ordered = useMemo(
    () => [...segments].sort((a, b) => a.start_sec - b.start_sec || a.end_sec - b.end_sec),
    // Content fingerprint, not array identity: the 1.2s project poll hands
    // us a brand-new segments[] every tick with identical cuts.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [segKey],
  )

  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [loop, setLoop] = useState(false)
  const [loadHint, setLoadHint] = useState<string | null>(null)
  /** Bump when proxy becomes ready so <video> reloads after warm encode. */
  const [videoGen, setVideoGen] = useState(0)
  /** Hold off attaching src until Chromium-safe proxy exists (HEVC sources). */
  const [playable, setPlayable] = useState(false)
  const { fps } = useVideoFps(projectId)

  /** Cache-bust on project change and when proxy flips ready. */
  const videoSrc = useMemo(
    () =>
      playable
        ? appendQuery(api.videoUrl(projectId), { v: `${projectId}-${videoGen}` })
        : null,
    [projectId, videoGen, playable],
  )

  // Reset to the first clip when the project or the cuts actually change.
  useEffect(() => {
    setActiveKey(ordered[0] ? segmentKey(ordered[0]) : null)
  }, [projectId, ordered])

  // Warm + poll proxy status. Do not await full encode (4K HEVC can take minutes).
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    let warmed = false
    let waiting = false
    setPlayable(false)
    setLoadHint("Loading video…")

    const tick = async () => {
      try {
        const st = await api.videoProxyStatus(projectId)
        if (cancelled) return
        if (st.native_ok || st.proxy_ready) {
          setLoadHint(null)
          setPlayable(true)
          if (waiting) setVideoGen((g) => g + 1)
          return
        }
        waiting = true
        if (st.build_error) {
          setLoadHint("Video preview failed to prepare")
          setPlayable(false)
          return
        }
        setLoadHint(
          st.building || st.needs_transcode
            ? "Preparing video preview (one-time)…"
            : "Preparing video preview…",
        )
        if (!warmed) {
          warmed = true
          // 202 Accepted — encode runs in a background thread on the API.
          void fetch(appendQuery(api.videoUrl(projectId), { warm: "1" })).catch(
            () => {},
          )
        }
        timer = setTimeout(() => {
          void tick()
        }, 2000)
      } catch {
        if (!cancelled) {
          setLoadHint(null)
          // Last resort: let <video> hit /video itself.
          setPlayable(true)
        }
      }
    }
    void tick()
    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [projectId])

  const activeIndex = Math.max(
    0,
    ordered.findIndex((s) => segmentKey(s) === activeKey),
  )
  const scene = ordered[activeIndex] ?? null

  // "Also in scene" only when another face window truly overlaps this clip —
  // never from time-union scenes (those glued whole dialogue exchanges).
  const coCastPool = allSegments?.length ? allSegments : segments
  const others = useMemo(
    () => (scene ? coCastForSegment(scene, coCastPool, people) : []),
    [scene, coCastPool, people],
  )
  const coCastByKey = useMemo(
    () => buildCoCastMap(ordered, coCastPool, people),
    [ordered, coCastPool, people],
  )

  const playerSegments = useMemo<PlayerSegment[]>(
    () =>
      ordered.map((s) => ({
        key: segmentKey(s),
        startSec: s.start_sec,
        endSec: s.end_sec,
      })),
    [ordered],
  )

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

  return (
    <div className={cn("min-w-0 max-w-full space-y-3 overflow-hidden", className)}>
      <VideoPlayer
        src={videoSrc}
        fps={fps}
        segments={playerSegments}
        mode="reel"
        activeKey={activeKey}
        onActiveChange={setActiveKey}
        loop={loop}
        onLoopChange={setLoop}
        controls="compact"
        stageClassName="mx-auto max-h-[200px] max-w-full rounded-lg border border-border/40 bg-[oklch(0.06_0.01_280)]"
      >
        {loadHint ? (
          <div className="absolute inset-0 z-[5] flex items-center justify-center bg-background/55 px-4 text-center">
            <p className="max-w-[16rem] text-[11px] leading-snug text-muted-foreground">
              {loadHint}
            </p>
          </div>
        ) : null}
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 bg-gradient-to-t from-background/95 via-background/35 to-transparent px-2.5 pb-2 pt-6">
          <p className="timecode text-[10px] tabular-nums text-primary">
            {String(activeIndex + 1).padStart(2, "0")}/
            {String(ordered.length).padStart(2, "0")}
          </p>
          <p className="timecode text-[10px] tabular-nums text-muted-foreground">
            {scene
              ? `${formatDuration(scene.start_sec)} - ${formatDuration(scene.end_sec)}`
              : "-"}
          </p>
        </div>
      </VideoPlayer>

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

      <div className="filmstrip-fade min-w-0 max-w-full">
        <div className="filmstrip-scroll flex max-w-full gap-1.5 pb-0.5">
          {ordered.map((s, i) => {
            const key = segmentKey(s)
            const active = i === activeIndex
            const castCount = 1 + (coCastByKey.get(key)?.length ?? 0)
            return (
              <button
                key={`${s.start_sec}-${s.end_sec}-${i}`}
                type="button"
                onClick={() => setActiveKey(key)}
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
