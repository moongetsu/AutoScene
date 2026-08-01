import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Maximize2, ZoomIn, ZoomOut } from "lucide-react"
import { formatTimecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"

export type TimelineView = { startSec: number; endSec: number }

export type TimelineBlock = {
  key: string
  startSec: number
  endSec: number
  excluded?: boolean
  manual?: boolean
  /** Outlined, not filled: a shot the pack could include but currently does not. */
  ghost?: boolean
  /** More than one id ⇒ shared shot; tints the block. */
  personIds?: number[]
  title?: string
}

export type TimelineLane = {
  id: string
  label: string
  personId?: number
  thumbnailUrl?: string | null
  blocks: TimelineBlock[]
  /** Renders muted, for the derived scenes strip. */
  muted?: boolean
}

export type TimelineSelection = { startSec: number; endSec: number }

type TimelineCanvasProps = {
  lanes: TimelineLane[]
  durationSec: number
  fps?: number | null
  view: TimelineView
  onViewChange: (view: TimelineView) => void
  playheadSec?: number
  onSeek?: (sec: number) => void
  activeKey?: string | null
  onSelectBlock?: (block: TimelineBlock, lane: TimelineLane) => void
  onSelectLane?: (lane: TimelineLane) => void
  selection?: TimelineSelection | null
  onSelectionChange?: (sel: TimelineSelection | null) => void
  onSelectionCommit?: (sel: TimelineSelection) => void
  /** Cut boundaries to snap drawn ranges to. Hold Alt to draw freehand. */
  snapPoints?: number[]
  className?: string
}

const RULER_H = 22
const LANE_H = 20
const LANE_GAP = 4
const LABEL_W = 116
/** Never zoom past this window; below it the ruler has nothing to say. */
const MIN_SPAN = 0.4
/** Pull a drawn edge to a cut within this many pixels. */
const SNAP_PX = 7

/** Nearest snap point within SNAP_PX, else the raw time. */
function snapTime(
  t: number,
  points: number[] | undefined,
  pxPerSec: number,
  enabled: boolean,
): number {
  if (!enabled || !points?.length || pxPerSec <= 0) return t
  const tol = SNAP_PX / pxPerSec
  let best = t
  let bestD = tol
  for (const p of points) {
    const d = Math.abs(p - t)
    if (d < bestD) {
      bestD = d
      best = p
    }
  }
  return best
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** Tick spacing that keeps labels ~70px apart at the current scale. */
function tickStep(pxPerSec: number): number {
  const target = 70 / Math.max(pxPerSec, 1e-6)
  const steps = [
    0.04, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600,
  ]
  return steps.find((s) => s >= target) ?? 3600
}

export function TimelineCanvas({
  lanes,
  durationSec,
  fps,
  view,
  onViewChange,
  playheadSec,
  onSeek,
  activeKey = null,
  onSelectBlock,
  onSelectLane,
  selection = null,
  onSelectionChange,
  onSelectionCommit,
  snapPoints,
  className,
}: TimelineCanvasProps) {
  const trackRef = useRef<HTMLDivElement | null>(null)
  const [width, setWidth] = useState(0)
  const [dragMode, setDragMode] = useState<"none" | "seek" | "select">("none")
  const dragAnchor = useRef(0)

  const duration = Math.max(durationSec, MIN_SPAN)
  const span = Math.max(MIN_SPAN, view.endSec - view.startSec)
  const pxPerSec = width > 0 ? width / span : 0

  useEffect(() => {
    const el = trackRef.current
    if (!el) return
    const ro = new ResizeObserver(([entry]) => {
      setWidth(entry.contentRect.width)
    })
    ro.observe(el)
    setWidth(el.getBoundingClientRect().width)
    return () => ro.disconnect()
  }, [])

  const timeToX = useCallback(
    (t: number) => (t - view.startSec) * pxPerSec,
    [view.startSec, pxPerSec],
  )
  const xToTime = useCallback(
    (clientX: number) => {
      const el = trackRef.current
      if (!el || pxPerSec <= 0) return view.startSec
      const rect = el.getBoundingClientRect()
      return clamp(view.startSec + (clientX - rect.left) / pxPerSec, 0, duration)
    },
    [view.startSec, pxPerSec, duration],
  )

  const setView = useCallback(
    (startSec: number, nextSpan: number) => {
      const s = clamp(nextSpan, MIN_SPAN, duration)
      const start = clamp(startSec, 0, duration - s)
      onViewChange({ startSec: start, endSec: start + s })
    },
    [duration, onViewChange],
  )

  const zoomAround = useCallback(
    (anchorSec: number, factor: number) => {
      const nextSpan = clamp(span / factor, MIN_SPAN, duration)
      const ratio = span > 0 ? (anchorSec - view.startSec) / span : 0.5
      setView(anchorSec - ratio * nextSpan, nextSpan)
    },
    [span, duration, view.startSec, setView],
  )

  // Wheel must be non-passive to preventDefault, which React's onWheel
  // cannot guarantee — attach manually.
  useEffect(() => {
    const el = trackRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      // Trackpad pinch arrives as wheel + ctrlKey.
      if (e.ctrlKey || e.metaKey || (!e.shiftKey && Math.abs(e.deltaY) > Math.abs(e.deltaX))) {
        e.preventDefault()
        const rect = el.getBoundingClientRect()
        const anchor = clamp(
          view.startSec + (e.clientX - rect.left) / Math.max(pxPerSec, 1e-6),
          0,
          duration,
        )
        zoomAround(anchor, Math.exp(-e.deltaY * 0.002))
      } else {
        e.preventDefault()
        const delta = (e.shiftKey ? e.deltaY : e.deltaX) / Math.max(pxPerSec, 1e-6)
        setView(view.startSec + delta, span)
      }
    }
    el.addEventListener("wheel", onWheel, { passive: false })
    return () => el.removeEventListener("wheel", onWheel)
  }, [view.startSec, span, pxPerSec, duration, zoomAround, setView])

  const ticks = useMemo(() => {
    if (pxPerSec <= 0) return []
    const step = tickStep(pxPerSec)
    const first = Math.ceil(view.startSec / step) * step
    const out: number[] = []
    for (let t = first; t <= view.endSec + 1e-9 && out.length < 200; t += step) {
      out.push(t)
    }
    return out
  }, [pxPerSec, view.startSec, view.endSec])

  // --- drag: seek on the ruler, range-select on the selection strip ------
  const beginSeek = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId)
    setDragMode("seek")
    onSeek?.(xToTime(e.clientX))
  }
  const beginSelect = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId)
    // Alt draws freehand; otherwise edges land on real cuts, which is what
    // makes a hand-drawn range usable as a pack clip.
    const t = snapTime(xToTime(e.clientX), snapPoints, pxPerSec, !e.altKey)
    dragAnchor.current = t
    setDragMode("select")
    onSelectionChange?.({ startSec: t, endSec: t })
  }
  const onDragMove = (e: React.PointerEvent) => {
    if (dragMode === "seek") {
      onSeek?.(xToTime(e.clientX))
    } else if (dragMode === "select") {
      const t = snapTime(xToTime(e.clientX), snapPoints, pxPerSec, !e.altKey)
      const a = Math.min(dragAnchor.current, t)
      const b = Math.max(dragAnchor.current, t)
      onSelectionChange?.({ startSec: a, endSec: b })
    }
  }
  const endDrag = (e: React.PointerEvent) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId)
    }
    if (dragMode === "select" && selection) {
      // Ignore accidental click-without-drag.
      if (selection.endSec - selection.startSec >= 0.08) onSelectionCommit?.(selection)
      else onSelectionChange?.(null)
    }
    setDragMode("none")
  }

  const trackH = RULER_H + lanes.length * (LANE_H + LANE_GAP)
  const zoomPct = duration > 0 ? duration / span : 1

  return (
    <div className={cn("flex min-w-0 flex-col gap-2", className)}>
      <div className="flex items-center gap-1.5">
        <button
          type="button"
          title="Zoom out"
          aria-label="Zoom out"
          onClick={() => zoomAround(view.startSec + span / 2, 1 / 1.8)}
          className="timeline-tool"
        >
          <ZoomOut className="size-3.5" />
        </button>
        <button
          type="button"
          title="Zoom in"
          aria-label="Zoom in"
          onClick={() => zoomAround(view.startSec + span / 2, 1.8)}
          className="timeline-tool"
        >
          <ZoomIn className="size-3.5" />
        </button>
        <button
          type="button"
          title="Fit whole video"
          aria-label="Fit whole video"
          onClick={() => setView(0, duration)}
          className="timeline-tool"
        >
          <Maximize2 className="size-3.5" />
        </button>
        <span className="timecode text-[10px] tabular-nums text-muted-foreground">
          {zoomPct < 1.05 ? "Full" : `${zoomPct.toFixed(zoomPct < 10 ? 1 : 0)}×`}
          <span className="mx-1.5 opacity-40">·</span>
          {formatTimecode(view.startSec, fps ?? 0)} – {formatTimecode(view.endSec, fps ?? 0)}
        </span>
        <span className="ml-auto text-[10px] text-muted-foreground/70">
          Scroll to zoom · Shift-scroll to pan · Drag the Manual strip to mark a range
        </span>
      </div>

      <div className="grid" style={{ gridTemplateColumns: `${LABEL_W}px minmax(0,1fr)` }}>
        {/* Label column — heights mirror the track column exactly. */}
        <div className="pr-2">
          <div style={{ height: RULER_H }} />
          {lanes.map((lane) => (
            <div
              key={lane.id}
              style={{ height: LANE_H, marginBottom: LANE_GAP }}
              className="flex items-center gap-1.5 overflow-hidden"
            >
              {lane.thumbnailUrl ? (
                <img
                  src={lane.thumbnailUrl}
                  alt=""
                  className="size-4 shrink-0 rounded-[2px] object-cover"
                />
              ) : null}
              <button
                type="button"
                onClick={() => onSelectLane?.(lane)}
                disabled={!onSelectLane}
                className={cn(
                  "min-w-0 truncate text-left text-[10px] transition",
                  lane.muted ? "text-muted-foreground/70" : "text-muted-foreground",
                  onSelectLane && "hover:text-foreground",
                )}
              >
                {lane.label}
              </button>
            </div>
          ))}
        </div>

        <div
          ref={trackRef}
          className="relative select-none overflow-hidden"
          style={{ height: trackH }}
        >
          {/* Ruler doubles as the scrub strip. */}
          <div
            className="absolute inset-x-0 top-0 cursor-text border-b border-border/40"
            style={{ height: RULER_H }}
            onPointerDown={beginSeek}
            onPointerMove={onDragMove}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
          >
            {ticks.map((t) => (
              <div
                key={t}
                className="absolute bottom-0 top-0 border-l border-border/45"
                style={{ left: timeToX(t) }}
              >
                <span className="timecode absolute left-1 top-0.5 whitespace-nowrap text-[9px] tabular-nums text-muted-foreground/80">
                  {formatTimecode(t, fps ?? 0)}
                </span>
              </div>
            ))}
          </div>

          {lanes.map((lane, laneIdx) => {
            const isSelectionStrip = lane.id === "manual"
            return (
              <div
                key={lane.id}
                className={cn("lane-row absolute", isSelectionStrip && "cursor-crosshair")}
                style={{
                  top: RULER_H + laneIdx * (LANE_H + LANE_GAP),
                  height: LANE_H,
                  left: 0,
                  right: 0,
                }}
                onPointerDown={isSelectionStrip ? beginSelect : undefined}
                onPointerMove={isSelectionStrip ? onDragMove : undefined}
                onPointerUp={isSelectionStrip ? endDrag : undefined}
                onPointerCancel={isSelectionStrip ? endDrag : undefined}
              >
                {/* Only draw what is on screen: a 45-min cast is thousands
                    of blocks and zoom would crawl without this. */}
                {lane.blocks.map((b) => {
                  if (b.endSec < view.startSec || b.startSec > view.endSec) return null
                  const left = timeToX(b.startSec)
                  const w = Math.max(2, (b.endSec - b.startSec) * pxPerSec)
                  const shared = (b.personIds?.length ?? 0) > 1
                  return (
                    <button
                      key={b.key}
                      type="button"
                      title={b.title}
                      aria-label={b.title ?? `Block ${b.key}`}
                      onPointerDown={(e) => e.stopPropagation()}
                      onClick={() => onSelectBlock?.(b, lane)}
                      className={cn(
                        "lane-block",
                        b.key === activeKey && "lane-block-active",
                        b.excluded && "lane-block-excluded",
                        b.manual && "lane-block-manual",
                        b.ghost && "lane-block-ghost",
                        !b.manual && !b.excluded && !b.ghost && shared && "lane-block-shared",
                        lane.muted && "opacity-70",
                      )}
                      style={{ left, width: w }}
                    />
                  )
                })}
              </div>
            )
          })}

          {selection && selection.endSec > selection.startSec ? (
            <div
              className="timeline-selection"
              style={{
                left: timeToX(selection.startSec),
                width: Math.max(2, (selection.endSec - selection.startSec) * pxPerSec),
                height: trackH,
              }}
            />
          ) : null}

          {playheadSec !== undefined &&
          playheadSec >= view.startSec &&
          playheadSec <= view.endSec ? (
            <div
              className="timeline-playhead"
              style={{ left: timeToX(playheadSec), height: trackH }}
            />
          ) : null}
        </div>
      </div>
    </div>
  )
}
