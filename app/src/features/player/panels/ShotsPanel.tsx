import { useCallback, useMemo } from "react"
import { ArrowCounterClockwise, CaretLeft, CaretRight, Copy, FilmStrip, Scissors } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem, Segment } from "@/bindings"
import { Button } from "@/components/ui/button"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { Kbd } from "@/components/ui/kbd"
import { Progress } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { formatPercent } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { copyText } from "@/lib/os"
import { nextShotStart, prevShotStart, segmentIndexAt, shotNumbers } from "@/lib/segments"
import { useShortcutKeys } from "@/lib/shortcuts"
import { useMediaJob } from "@/state/activity"
import { type FrameStore, useFrameSelector } from "../frame-store"
import { SEGMENT_LABEL } from "../labels"
import { Card } from "./Card"

/** The shot under the playhead, or where shot detection stands. It draws again only when the shot changes. */
export function ShotsPanel({
  pid,
  media: m,
  segments,
  store,
  tc,
  onMark,
  onGo,
}: {
  pid: string
  media: MediaItem
  segments: Segment[]
  store: FrameStore
  tc: (f: number) => string
  onMark: () => void
  onGo: (f: number) => void
}) {
  const keys = useShortcutKeys()
  const job = useMediaJob(m.id, "analyze")
  const numbers = useMemo(() => shotNumbers(segments), [segments])
  const total = numbers.reduce((n, x) => (x >= 0 ? n + 1 : n), 0)
  const i = useFrameSelector(
    store,
    useCallback((f: number) => segmentIndexAt(segments, f), [segments]),
  )
  const seg = i >= 0 ? segments[i] : null
  const go = (to: number | null) => to != null && onGo(to)
  const analyze = () =>
    void commands
      .mediaAnalyze(pid, m.id, null)
      .catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))

  const menu =
    m.shots === "ready" && seg ? (
      <>
        {seg.kind === "shot" && (
          <ContextMenuItem onSelect={onMark} shortcut={keys("player.markShot")}>
            <FilmStrip /> Mark the shot as the clip
          </ContextMenuItem>
        )}
        <ContextMenuItem onSelect={() => go(prevShotStart(segments, store.get()))} shortcut={keys("player.shot", 0)}>
          <CaretLeft /> Previous shot
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => go(nextShotStart(segments, store.get()))} shortcut={keys("player.shot", 1)}>
          <CaretRight /> Next shot
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => void copyText(`${tc(seg.start)} - ${tc(seg.end - 1)}`, "timecodes")}>
          <Copy /> Copy its timecodes
        </ContextMenuItem>
      </>
    ) : (
      !job && (
        <ContextMenuItem onSelect={analyze}>
          {m.shots === "failed" ? <ArrowCounterClockwise /> : <Scissors />} {m.shots === "failed" ? "Find shots again" : "Find shots"}
        </ContextMenuItem>
      )
    )

  return (
    <Card title="Shot" menu={menu} action={m.shots === "ready" && <span className="timecode text-xs text-fg-3">{total} shots</span>}>
      {m.shots === "ready" && seg ? (
        <>
          <div className="flex items-baseline justify-between gap-2">
            <span className="text-lg font-medium text-fg-1">
              {seg.kind === "shot" ? `Shot ${numbers[i] + 1}` : SEGMENT_LABEL[seg.kind]}
              {seg.tiny && <span className="text-fg-3"> · tiny</span>}
            </span>
            <span className="timecode text-sm text-fg-2">{seg.end - seg.start} frames</span>
          </div>
          <p className="timecode mt-0.5 text-xs text-fg-3">
            {tc(seg.start)} – {tc(seg.end - 1)}
          </p>
          <div className="mt-3 flex gap-2">
            <Button size="sm" variant="secondary" className="flex-1" onClick={onMark}>
              Mark shot <Kbd keys={keys("player.markShot")} className="ml-auto" />
            </Button>
            <Tooltip label="Previous shot" shortcut={keys("player.shot", 0)}>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="Previous shot"
                onClick={() => go(prevShotStart(segments, store.get()))}
              >
                <CaretLeft />
              </Button>
            </Tooltip>
            <Tooltip label="Next shot" shortcut={keys("player.shot", 1)}>
              <Button size="icon-sm" variant="ghost" aria-label="Next shot" onClick={() => go(nextShotStart(segments, store.get()))}>
                <CaretRight />
              </Button>
            </Tooltip>
          </div>
        </>
      ) : job ? (
        <div>
          <div className="flex justify-between text-xs text-fg-3">
            <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "Finding shots")}</span>
            <span className="timecode">{formatPercent(job.progress)}</span>
          </div>
          <Progress className="mt-1.5" value={job.state === "running" ? job.progress : null} />
        </div>
      ) : m.shots === "failed" ? (
        <div>
          <p className="text-sm text-danger">Shot detection failed.</p>
          {m.shotsError && <p className="mt-0.5 text-xs break-words text-fg-3">{m.shotsError}</p>}
          <Button size="sm" variant="ghost" className="mt-2 -ml-2" onClick={analyze}>
            <ArrowCounterClockwise /> Try again
          </Button>
        </div>
      ) : (
        <div>
          <p className="text-sm text-fg-3">Shots haven't been detected yet.</p>
          <Button size="sm" variant="ghost" className="mt-2 -ml-2" onClick={analyze}>
            <Scissors /> Find shots
          </Button>
        </div>
      )}
    </Card>
  )
}
