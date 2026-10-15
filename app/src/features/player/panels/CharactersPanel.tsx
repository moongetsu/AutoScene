import { useCallback, useState } from "react"
import { Link } from "@tanstack/react-router"
import { Users } from "@/components/icons"
import type { MediaItem, Segment } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { formatPercent } from "@/lib/format"
import { assetUrl } from "@/lib/media"
import { type Lane, charactersIn } from "@/lib/presence"
import { shotAt } from "@/lib/segments"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"
import { type FrameStore, useFrameSelector } from "../frame-store"
import { Card } from "./Card"

/**
 * Who is in the shot under the playhead, and everyone in this episode. It
 * draws again when who is on screen changes, not on every frame.
 */
export function CharactersPanel({
  pid,
  media: m,
  lanes,
  segments,
  store,
  focus,
  onFocus,
  onGo,
}: {
  pid: string
  media: MediaItem
  lanes: Lane[]
  segments: Segment[]
  store: FrameStore
  focus: string | null
  onFocus: (id: string | null) => void
  onGo: (f: number) => void
}) {
  const job = useMediaJob(m.id, "analyze")
  const [all, setAll] = useState(false)
  // Who is on screen: in the shot under the playhead (or at the frame, before shots are found).
  const hereKey = useFrameSelector(
    store,
    useCallback(
      (f: number) => {
        const shot = shotAt(segments, f)
        const on = shot ? charactersIn(lanes, shot.start, shot.end) : charactersIn(lanes, f, f + 1)
        return on.map((l) => l.character.id).join(",")
      },
      [lanes, segments],
    ),
  )
  const hereIds = new Set(hereKey ? hereKey.split(",") : [])
  const here = lanes.filter((l) => hereIds.has(l.character.id))
  // Who is in the shot, then the episode's main characters; the rest on request.
  const others = lanes.filter((l) => !hereIds.has(l.character.id))
  const listed = [...here, ...(all ? others : others.slice(0, Math.max(0, 8 - here.length)))]
  const hiddenCount = here.length + others.length - listed.length
  /** The next time `lane` is on screen after the playhead (wrapping around). */
  const next = (lane: Lane) => {
    const frame = store.get()
    return (lane.spans.find((s) => s.start > frame) ?? lane.spans[0])?.start
  }
  return (
    <Card
      title="Characters"
      action={
        <Link to="/p/$pid/cast" params={{ pid }} className="text-xs font-medium text-fg-3 hover:text-fg-1">
          Cast
        </Link>
      }
    >
      {m.faces !== "ready" ? (
        job ? (
          <div>
            <div className="flex justify-between text-xs text-fg-3">
              <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "Analysing")}</span>
              <span className="timecode">{formatPercent(job.progress)}</span>
            </div>
            <Progress className="mt-1.5" value={job.state === "running" ? job.progress : null} />
          </div>
        ) : (
          <p className="text-sm text-fg-3">
            {m.faces === "failed" ? (m.facesError ?? "Finding characters failed.") : "Characters haven't been found in this episode yet."}
          </p>
        )
      ) : lanes.length === 0 ? (
        <p className="text-sm text-fg-3">Nobody the cast knows appears here.</p>
      ) : (
        <ul className="-mx-1.5 flex flex-col gap-0.5">
          {listed.map((l) => {
            const c = l.character
            const on = hereIds.has(c.id)
            return (
              <li key={c.id}>
                <button
                  onClick={() => onFocus(focus === c.id ? null : c.id)}
                  onDoubleClick={() => {
                    const f = next(l)
                    if (f != null) onGo(f)
                  }}
                  title="Click to highlight · double-click for the next appearance"
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-xl px-1.5 py-1.5 text-left transition-colors hover:bg-white/[0.06]",
                    focus === c.id && "bg-accent-subtle",
                  )}
                >
                  <span className={cn("size-8 shrink-0 overflow-hidden rounded-full bg-inset", on ? "ring-2 ring-accent/70" : "opacity-50")}>
                    {c.coverSmall && <img src={assetUrl(c.coverSmall)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className={cn("min-w-0 flex-1 truncate text-sm font-medium", on ? "text-fg-1" : "text-fg-3")}>{c.label}</span>
                  {on && <span className="text-xs text-accent">in shot</span>}
                </button>
              </li>
            )
          })}
          {(hiddenCount > 0 || all) && (
            <li>
              <button
                onClick={() => setAll((v) => !v)}
                className="mt-0.5 w-full rounded-xl px-1.5 py-1.5 text-left text-xs font-medium text-fg-3 hover:bg-white/[0.06] hover:text-fg-1"
              >
                {all ? "Show fewer" : `${hiddenCount} more in this episode`}
              </button>
            </li>
          )}
        </ul>
      )}
      {m.faces === "ready" && lanes.length > 0 && (
        <Button size="sm" variant="ghost" className="mt-2 -ml-2" asChild>
          <Link to="/p/$pid/cast" params={{ pid }}>
            <Users /> Edit the cast
          </Link>
        </Button>
      )}
    </Card>
  )
}
