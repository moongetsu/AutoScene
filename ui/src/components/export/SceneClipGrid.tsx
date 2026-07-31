import { useEffect, useState } from "react"
import { motion } from "motion/react"
import { EyeOff } from "lucide-react"
import {
  api,
  appendQuery,
  formatDuration,
  isSegmentExcluded,
  type Person,
  type Scene,
  type Segment,
  type SegmentExclusion,
} from "@/lib/api"
import { SceneClipPreview } from "@/components/export/SceneClipPreview"
import { coCastForSegment, segmentPreviewRange } from "@/lib/scenes"
import { cn } from "@/lib/utils"

type SceneClipGridProps = {
  projectId?: string
  segments: Segment[]
  /** Full project segments — needed for honest co-cast overlap. */
  allSegments?: Segment[]
  exclusions: SegmentExclusion[]
  people?: Person[]
  scenes?: Scene[]
  disabled?: boolean
  onToggle: (segment: Segment, excluded: boolean) => void
}

export function SceneClipGrid({
  projectId,
  segments,
  allSegments,
  exclusions,
  people = [],
  scenes: _scenes = [],
  disabled,
  onToggle,
}: SceneClipGridProps) {
  const [hoverVideoEnabled, setHoverVideoEnabled] = useState(false)
  const [proxyHint, setProxyHint] = useState<string | null>(null)

  // Warm browser-playable proxy once per project (HEVC sources need this for hover).
  useEffect(() => {
    if (!projectId) {
      setHoverVideoEnabled(false)
      setProxyHint(null)
      return
    }
    let cancelled = false
    setHoverVideoEnabled(false)
    setProxyHint(null)

    void (async () => {
      try {
        const st = await api.videoProxyStatus(projectId)
        if (cancelled) return
        if (st.native_ok || st.proxy_ready) {
          setHoverVideoEnabled(true)
          setProxyHint(null)
          return
        }
        setProxyHint("Preparing hover preview…")
        // Triggers ensure_browser_playable on the API (may re-encode once).
        const res = await fetch(
          appendQuery(api.videoUrl(projectId), { warm: "1" }),
          { headers: { Range: "bytes=0-1023" } },
        )
        if (cancelled) return
        if (res.ok || res.status === 206) {
          setHoverVideoEnabled(true)
          setProxyHint(null)
        } else {
          setProxyHint(null)
        }
      } catch {
        if (!cancelled) setProxyHint(null)
      }
    })()

    return () => {
      cancelled = true
    }
  }, [projectId])

  if (!segments.length) {
    return (
      <div className="flex min-h-[180px] items-center justify-center rounded-lg border border-[rgba(242,237,230,0.10)] bg-[#1C1A17] p-8 text-sm text-[#9A948A]">
        No scenes for this character yet. Run a scan first.
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {proxyHint ? (
        <p className="text-[11px] text-[#9A948A]">{proxyHint}</p>
      ) : null}
      <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
        {segments.map((segment, index) => {
          const excluded = isSegmentExcluded(exclusions, segment)
          const duration = segment.end_sec - segment.start_sec
          const others = coCastForSegment(
            segment,
            allSegments?.length ? allSegments : segments,
            people,
          )
          const preview = segmentPreviewRange(segment)

          return (
            <motion.div
              key={`${segment.start_sec}-${segment.end_sec}-${index}`}
              role="button"
              tabIndex={disabled ? -1 : 0}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{
                type: "spring",
                stiffness: 100,
                damping: 20,
                delay: Math.min(index, 12) * 0.025,
              }}
              onClick={() => {
                if (!disabled) onToggle(segment, !excluded)
              }}
              onKeyDown={(e) => {
                if (disabled) return
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault()
                  onToggle(segment, !excluded)
                }
              }}
              className={cn(
                "group relative cursor-pointer overflow-hidden rounded-lg border text-left transition duration-200",
                excluded
                  ? "border-border/25 bg-muted/15 opacity-55"
                  : "border-[rgba(242,237,230,0.10)] bg-[#23201C]/80 hover:border-[#C4A35A]/40 hover:bg-[#23201C]",
                disabled && "pointer-events-none cursor-default opacity-60",
              )}
            >
              <div className="relative aspect-[16/9] overflow-hidden bg-[#141210]">
                {projectId ? (
                  <SceneClipPreview
                    projectId={projectId}
                    startSec={preview.startSec}
                    endSec={preview.endSec}
                    faceTimeSec={segment.face_time_sec}
                    hoverVideoEnabled={hoverVideoEnabled}
                    className={cn(excluded && "opacity-70")}
                  />
                ) : null}

                <div className="pointer-events-none absolute left-2 top-2 z-[2] rounded border border-[rgba(242,237,230,0.10)] bg-[#141210]/85 px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-[#F2EDE6]/90">
                  {String(index + 1).padStart(2, "0")}
                </div>
                {excluded ? (
                  <div className="pointer-events-none absolute right-2 top-2 z-[2] inline-flex items-center gap-1 rounded border border-[#B54A3A]/40 bg-[#141210]/85 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-[#B54A3A]">
                    <EyeOff className="size-3" />
                    Off
                  </div>
                ) : null}
                <div className="pointer-events-none absolute inset-x-0 bottom-0 z-[2] bg-gradient-to-t from-[#141210]/95 via-[#141210]/45 to-transparent px-2.5 pb-2 pt-8">
                  <p className="font-mono text-xs tabular-nums text-[#F2EDE6]/90">
                    {formatDuration(segment.start_sec)} → {formatDuration(segment.end_sec)}
                  </p>
                  <p className="mt-0.5 font-mono text-[10px] tabular-nums text-[#9A948A]">
                    {formatDuration(duration)} · click to{" "}
                    {excluded ? "include" : "exclude"}
                  </p>
                </div>
              </div>

              {others.length > 0 ? (
                <div className="relative z-[1] flex items-center gap-1.5 border-t border-border/30 bg-card/80 px-2.5 py-2">
                  <span className="text-[9px] uppercase tracking-[0.14em] text-muted-foreground">
                    With
                  </span>
                  <div className="flex min-w-0 flex-1 items-center gap-1 overflow-hidden">
                    {others.slice(0, 4).map((p) => (
                      <span
                        key={p.person_id}
                        className="inline-flex max-w-[5.5rem] items-center gap-1 truncate rounded border border-border/35 bg-background/40 px-1 py-0.5 text-[10px]"
                      >
                        {projectId && p.thumbnail_path ? (
                          <img
                            src={api.mediaUrl(projectId, p.thumbnail_path)}
                            alt=""
                            className="size-3.5 rounded-sm object-cover"
                          />
                        ) : null}
                        <span className="truncate">{p.label}</span>
                      </span>
                    ))}
                    {others.length > 4 ? (
                      <span className="text-[10px] text-muted-foreground">
                        +{others.length - 4}
                      </span>
                    ) : null}
                  </div>
                </div>
              ) : null}
            </motion.div>
          )
        })}
      </div>
    </div>
  )
}
