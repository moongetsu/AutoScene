import { memo, useRef, useState } from "react"
import type { Presence } from "@/bindings"
import type { FrameClock } from "@/lib/frame-clock"
import { assetUrl } from "@/lib/media"
import type { Lane } from "@/lib/presence"
import { cn } from "@/lib/utils"

/** Lanes shown under the shot strip; everyone else is listed in the inspector. */
const MAX_LANES = 5

/**
 * One lane per main character on the scrubber's time axis: where their face
 * is on screen. Click a span to go there; the focused character is gold.
 */
export function CharacterLanes({
  lanes,
  clock,
  focus,
  onSeek,
  formatFrame,
}: {
  lanes: Lane[]
  clock: FrameClock
  focus: string | null
  onSeek: (f: number) => void
  formatFrame: (f: number) => string
}) {
  const last = Math.max(1, clock.frames - 1)
  const pct = (f: number) => Math.min(100, (clock.rel(f) / Math.max(1e-9, clock.rel(last))) * 100)
  const shown = lanes.slice(0, MAX_LANES)
  if (shown.length === 0) return null
  return (
    <div className="mt-1 flex flex-col gap-px" role="group" aria-label="Characters on screen">
      {shown.map((lane) => (
        <LaneRow
          key={lane.character.id}
          lane={lane}
          pct={pct}
          focused={focus === lane.character.id}
          onSeek={onSeek}
          formatFrame={formatFrame}
        />
      ))}
    </div>
  )
}

function LaneRow({
  lane,
  pct,
  focused,
  onSeek,
  formatFrame,
}: {
  lane: Lane
  pct: (f: number) => number
  focused: boolean
  onSeek: (f: number) => void
  formatFrame: (f: number) => string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; span: Presence } | null>(null)
  const c = lane.character
  const spanAt = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = clientX - r.left
    const at = (x / r.width) * 100
    // The span under the pointer, else the nearest one within a few pixels.
    let best: Presence | null = null
    let dist = (6 / r.width) * 100
    for (const s of lane.spans) {
      const a = pct(s.start)
      const b = pct(s.end)
      const d = at < a ? a - at : at > b ? at - b : 0
      if (d <= dist) {
        dist = d
        best = s
      }
    }
    return best ? { x, span: best } : null
  }
  return (
    <div
      ref={ref}
      className="group relative h-3 cursor-pointer rounded-[2px] bg-inset/60"
      onPointerMove={(e) => setHover(spanAt(e.clientX))}
      onPointerLeave={() => setHover(null)}
      onClick={(e) => {
        const h = spanAt(e.clientX)
        if (h) onSeek(h.span.start)
      }}
    >
      <Spans spans={lane.spans} pct={pct} focused={focused} />
      <span className="pointer-events-none absolute inset-y-0 left-0 flex items-center gap-1 rounded-l-[2px] bg-gradient-to-r from-panel via-panel/80 to-transparent pr-3 pl-0.5">
        {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-2.5 rounded-full object-cover" />}
        <span className={cn("text-[9px] leading-none", focused ? "text-accent" : "text-fg-3 group-hover:text-fg-2")}>
          {c.label}
        </span>
      </span>
      {hover && (
        <div
          className="glass pointer-events-none absolute bottom-full z-10 mb-1 -translate-x-1/2 rounded-sm px-1.5 py-0.5 text-2xs whitespace-nowrap"
          style={{ left: hover.x }}
        >
          <span className="text-fg-1">{c.label}</span>
          <span className="timecode text-fg-3"> · {formatFrame(hover.span.start)}</span>
        </div>
      )}
    </div>
  )
}

const Spans = memo(function Spans({
  spans,
  pct,
  focused,
}: {
  spans: Presence[]
  pct: (f: number) => number
  focused: boolean
}) {
  return (
    <>
      {spans.map((s) => {
        const left = pct(s.start)
        return (
          <div
            key={s.trackId}
            className={cn("absolute inset-y-0.5 rounded-[1px]", focused ? "bg-accent" : "bg-fg-3/70")}
            style={{ left: `${left}%`, width: `${Math.max(0.12, pct(s.end) - left)}%` }}
          />
        )
      })}
    </>
  )
}, (a, b) => a.spans === b.spans && a.focused === b.focused)
