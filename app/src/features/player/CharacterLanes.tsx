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
    <div className="mt-1 flex flex-col gap-1" role="group" aria-label="Characters on screen">
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
    <div className="group grid grid-cols-[var(--lane-label-w)_minmax(0,1fr)] items-center gap-3">
      <span className="flex min-w-0 items-center gap-2">
        {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-5 shrink-0 rounded-full object-cover" />}
        <span
          className={cn(
            "truncate text-xs font-medium",
            focused ? "text-accent" : c.name ? "text-fg-2 group-hover:text-fg-1" : "text-fg-3 group-hover:text-fg-2",
          )}
        >
          {c.label}
        </span>
      </span>
      <div
        ref={ref}
        className="relative h-6 rounded-lg bg-white/[0.03]"
        onPointerMove={(e) => setHover(spanAt(e.clientX))}
        onPointerLeave={() => setHover(null)}
        onClick={(e) => {
          const h = spanAt(e.clientX)
          if (h) onSeek(h.span.start)
        }}
      >
        <Spans spans={lane.spans} pct={pct} focused={focused} />
        {hover && (
          <div
            className="glass-strong pointer-events-none absolute bottom-full z-10 mb-1.5 -translate-x-1/2 rounded-lg px-2 py-1 text-xs whitespace-nowrap"
            style={{ left: hover.x }}
          >
            <span className="text-fg-1">{c.label}</span>
            <span className="timecode text-fg-3"> · {formatFrame(hover.span.start)}</span>
          </div>
        )}
      </div>
    </div>
  )
}

const Spans = memo(
  function Spans({ spans, pct, focused }: { spans: Presence[]; pct: (f: number) => number; focused: boolean }) {
    return (
      <>
        {spans.map((s) => {
          const left = pct(s.start)
          return (
            <div
              key={s.trackId}
              className={cn(
                "absolute inset-y-[7px] rounded-full",
                focused ? "bg-accent shadow-[0_0_8px_var(--accent)]" : "bg-white/40",
              )}
              style={{ left: `${left}%`, width: `${Math.max(0.12, pct(s.end) - left)}%` }}
            />
          )
        })}
      </>
    )
  },
  (a, b) => a.spans === b.spans && a.focused === b.focused,
)
