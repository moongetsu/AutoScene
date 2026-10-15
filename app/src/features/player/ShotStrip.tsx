import { memo, useMemo, useRef, useState } from "react"
import type { Segment } from "@/bindings"
import type { FrameClock } from "@/lib/frame-clock"
import { segmentIndexAt, shotNumbers } from "@/lib/segments"
import { cn } from "@/lib/utils"

const KIND_LABEL: Record<Segment["kind"], string> = {
  shot: "Shot",
  dissolve: "Dissolve",
  fade_out: "Fade out",
  fade_in: "Fade in",
  black: "Black",
  flash: "Flash",
}

/**
 * The media's shots under the scrubber, on the same time axis: shots as
 * blocks, transitions hatched, black frames dark. The shot under the
 * playhead is outlined. Click a shot to go to its first frame; double-click
 * marks it as the clip.
 */
export function ShotStrip({
  segments,
  clock,
  frame,
  onSeek,
  onMarkShot,
  formatFrame,
}: {
  segments: Segment[]
  clock: FrameClock
  frame: number
  onSeek: (f: number) => void
  onMarkShot: (s: Segment) => void
  formatFrame: (f: number) => string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; i: number } | null>(null)
  const numbers = useMemo(() => shotNumbers(segments), [segments])
  const last = Math.max(1, clock.frames - 1)
  // Same mapping as the scrubber (time-proportional for VFR media).
  const pct = (f: number) => Math.min(100, (clock.rel(f) / Math.max(1e-9, clock.rel(last))) * 100)
  const current = segmentIndexAt(segments, frame)
  const cur = current >= 0 ? segments[current] : null

  const indexAtX = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = Math.min(Math.max(clientX - r.left, 0), r.width)
    const f = clock.frameAt(clock.start(0) + (x / r.width) * clock.rel(last))
    return { x, i: segmentIndexAt(segments, f) }
  }

  const hovered = hover && hover.i >= 0 ? segments[hover.i] : null
  const seconds = (s: Segment) => (clock.rel(s.end) - clock.rel(s.start)).toFixed(1)

  return (
    <div
      ref={ref}
      className="relative h-5 rounded-md select-none"
      role="group"
      aria-label="Shots"
      onPointerMove={(e) => setHover(indexAtX(e.clientX))}
      onPointerLeave={() => setHover(null)}
      onClick={(e) => {
        const { i } = indexAtX(e.clientX)
        if (i >= 0) onSeek(segments[i].start)
      }}
      onDoubleClick={(e) => {
        const { i } = indexAtX(e.clientX)
        if (i >= 0 && segments[i].kind === "shot") onMarkShot(segments[i])
      }}
    >
      <Blocks segments={segments} numbers={numbers} pct={pct} />
      {cur && (
        <div
          className="pointer-events-none absolute inset-y-0 rounded-[3px] bg-accent/25 ring-1 ring-accent ring-inset"
          style={{ left: `${pct(cur.start)}%`, width: `${Math.max(0.15, pct(cur.end) - pct(cur.start))}%` }}
        />
      )}
      {hover && hovered && (
        <div
          className="glass-strong pointer-events-none absolute bottom-full mb-2 -translate-x-1/2 rounded-lg px-2 py-1 text-xs whitespace-nowrap"
          style={{ left: hover.x }}
        >
          <span className="text-fg-1">
            {hovered.kind === "shot" ? `Shot ${numbers[hover.i] + 1}` : KIND_LABEL[hovered.kind]}
          </span>
          <span className="timecode text-fg-3">
            {" "}
            · {formatFrame(hovered.start)} · {seconds(hovered)} s{hovered.tiny ? " · tiny" : ""}
          </span>
        </div>
      )}
    </div>
  )
}

/** The blocks only change with the segments, not with the playhead. */
const Blocks = memo(function Blocks({
  segments,
  numbers,
  pct,
}: {
  segments: Segment[]
  numbers: number[]
  pct: (f: number) => number
}) {
  return (
    <>
      {segments.map((s, i) => {
        const left = pct(s.start)
        const width = Math.max(0.05, pct(s.end) - left)
        return (
          <div
            key={s.start}
            className={cn(
              "absolute inset-y-0 border-r border-black/50",
              s.kind === "shot" && (numbers[i] % 2 === 0 ? "bg-white/[0.09]" : "bg-white/[0.15]"),
              s.kind === "black" && "bg-inset",
              s.kind === "flash" && "bg-fg-2",
              (s.kind === "dissolve" || s.kind === "fade_in" || s.kind === "fade_out") &&
                "bg-[repeating-linear-gradient(135deg,rgb(255_255_255/0.22)_0_2px,transparent_2px_4px)]",
            )}
            style={{ left: `${left}%`, width: `${width}%` }}
          />
        )
      })}
    </>
  )
}, (a, b) => a.segments === b.segments && a.numbers === b.numbers)
