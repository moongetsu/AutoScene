import { useRef, useState } from "react"
import type { FrameClock } from "@/lib/frame-clock"

/**
 * Seek bar: played region, marked in/out range (gold), playhead, and a
 * timecode tooltip under the pointer. Dragging snaps to whole frames.
 */
export function Scrubber({
  clock,
  frame,
  inFrame,
  outFrame,
  onSeek,
  onScrubStart,
  formatFrame,
}: {
  clock: FrameClock
  frame: number
  inFrame: number | null
  outFrame: number | null
  onSeek: (f: number) => void
  onScrubStart: () => void
  formatFrame: (f: number) => string
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; f: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  const last = Math.max(1, clock.frames - 1)

  // Positions by time, so VFR media scrub proportionally to real time.
  const pct = (f: number) => (clock.rel(f) / Math.max(1e-9, clock.rel(last))) * 100
  const frameAtX = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = Math.min(Math.max(clientX - r.left, 0), r.width)
    return { x, f: clock.frameAt(clock.start(0) + (x / r.width) * clock.rel(last)) }
  }

  const start = inFrame ?? 0
  const end = outFrame ?? last
  const hasRange = inFrame != null || outFrame != null

  return (
    <div
      ref={ref}
      role="slider"
      tabIndex={0}
      aria-label="Playhead"
      aria-valuemin={0}
      aria-valuemax={last}
      aria-valuenow={frame}
      aria-valuetext={formatFrame(frame)}
      className="group/scrub relative h-9 touch-none rounded-lg select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-subtle"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId)
        setDragging(true)
        onScrubStart()
        onSeek(frameAtX(e.clientX).f)
      }}
      onPointerMove={(e) => {
        const h = frameAtX(e.clientX)
        setHover(h)
        if (dragging) onSeek(h.f)
      }}
      onPointerUp={(e) => {
        e.currentTarget.releasePointerCapture(e.pointerId)
        setDragging(false)
      }}
      onPointerLeave={() => !dragging && setHover(null)}
    >
      {/* Track */}
      <div className="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 overflow-hidden rounded-full bg-white/10 transition-[height] duration-[var(--dur-fast)] group-hover/scrub:h-2">
        <div className="absolute inset-y-0 left-0 bg-white/35" style={{ width: `${pct(frame)}%` }} />
        {hasRange && (
          <div
            className="absolute inset-y-0 bg-accent/60 shadow-[0_0_10px_var(--accent)]"
            style={{ left: `${pct(start)}%`, width: `${Math.max(0.2, pct(end) - pct(start))}%` }}
          />
        )}
      </div>
      {/* In / out brackets */}
      {inFrame != null && (
        <div className="absolute top-1.5 bottom-1.5 w-[4px] rounded-l-[3px] border-y-2 border-l-2 border-accent" style={{ left: `${pct(inFrame)}%` }} />
      )}
      {outFrame != null && (
        <div
          className="absolute top-1.5 bottom-1.5 w-[4px] -translate-x-full rounded-r-[3px] border-y-2 border-r-2 border-accent"
          style={{ left: `${pct(outFrame)}%` }}
        />
      )}
      {/* Playhead */}
      <div className="pointer-events-none absolute top-0 bottom-0 -translate-x-1/2" style={{ left: `${pct(frame)}%` }}>
        <div className="mx-auto h-full w-[2px] rounded-full bg-white shadow-[0_0_8px_rgb(255_255_255/0.6)]" />
        <div className="absolute top-1/2 left-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_0_0_3px_rgb(0_0_0/0.35),0_0_12px_rgb(255_255_255/0.5)]" />
      </div>
      {/* Hover timecode */}
      {hover && (
        <div
          className="glass-strong pointer-events-none absolute bottom-full mb-2 -translate-x-1/2 rounded-lg px-2 py-1 text-xs whitespace-nowrap"
          style={{ left: hover.x }}
        >
          <span className="timecode text-fg-1">{formatFrame(hover.f)}</span>
        </div>
      )}
    </div>
  )
}
