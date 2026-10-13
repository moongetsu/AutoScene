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
      className="relative h-7 cursor-pointer touch-none rounded-sm select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-subtle"
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
      <div className="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 overflow-hidden rounded-full bg-active">
        <div className="absolute inset-y-0 left-0 bg-strong" style={{ width: `${pct(frame)}%` }} />
        {hasRange && (
          <div
            className="absolute inset-y-0 bg-accent/45"
            style={{ left: `${pct(start)}%`, width: `${Math.max(0.2, pct(end) - pct(start))}%` }}
          />
        )}
      </div>
      {/* In / out brackets */}
      {inFrame != null && (
        <div className="absolute top-1 bottom-1 w-[3px] rounded-l-xs border-y-2 border-l-2 border-accent" style={{ left: `${pct(inFrame)}%` }} />
      )}
      {outFrame != null && (
        <div
          className="absolute top-1 bottom-1 w-[3px] -translate-x-full rounded-r-xs border-y-2 border-r-2 border-accent"
          style={{ left: `${pct(outFrame)}%` }}
        />
      )}
      {/* Playhead */}
      <div className="pointer-events-none absolute top-0 bottom-0 -translate-x-1/2" style={{ left: `${pct(frame)}%` }}>
        <div className="mx-auto h-full w-[2px] rounded-full bg-fg-1 shadow-[0_0_0_1px_rgb(0_0_0/0.5)]" />
      </div>
      {/* Hover timecode */}
      {hover && (
        <div
          className="glass pointer-events-none absolute bottom-full mb-1.5 -translate-x-1/2 rounded-sm px-1.5 py-0.5 text-2xs whitespace-nowrap"
          style={{ left: hover.x }}
        >
          <span className="timecode text-fg-1">{formatFrame(hover.f)}</span>
        </div>
      )}
    </div>
  )
}
