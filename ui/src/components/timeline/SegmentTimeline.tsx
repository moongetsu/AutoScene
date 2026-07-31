import type { Segment } from "@/lib/api"
import { formatDuration } from "@/lib/api"
import { cn } from "@/lib/utils"

type SegmentTimelineProps = {
  segments: Segment[]
  durationSec: number
  activeIndex?: number | null
  onSelect?: (index: number) => void
}

export function SegmentTimeline({
  segments,
  durationSec,
  activeIndex = null,
  onSelect,
}: SegmentTimelineProps) {
  const dur = Math.max(durationSec, 1)

  return (
    <div className="space-y-2">
      <div className="segment-bar">
        {segments.map((s, i) => {
          const left = (s.start_sec / dur) * 100
          const width = Math.max(0.35, ((s.end_sec - s.start_sec) / dur) * 100)
          return (
            <button
              key={`${s.start_sec}-${i}`}
              type="button"
              aria-label={`Segment ${i + 1}`}
              className={cn(
                "segment-bar-fill cursor-pointer",
                activeIndex === i && "segment-bar-fill-active",
              )}
              style={{ left: `${left}%`, width: `${width}%` }}
              onClick={() => onSelect?.(i)}
            />
          )
        })}
      </div>
      <div className="flex justify-between text-[10px] uppercase tracking-wider text-muted-foreground">
        <span className="timecode">0:00</span>
        <span className="timecode">{formatDuration(durationSec)}</span>
      </div>
    </div>
  )
}
