import type * as React from "react"
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import {
  ArrowLineLeft,
  ArrowLineRight,
  ArrowsOutLineHorizontal,
  Copy,
  Crosshair,
  FilmStrip,
  MagnifyingGlassMinus,
  MagnifyingGlassPlus,
  SkipForward,
  User,
  UserFocus,
} from "@/components/icons"
import type { Presence, Segment } from "@/bindings"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuLabel, ContextMenuSeparator } from "@/components/ui/context-menu"
import { Tooltip } from "@/components/ui/tooltip"
import type { FrameClock } from "@/lib/frame-clock"
import { assetUrl } from "@/lib/media"
import { copyText } from "@/lib/os"
import { type Lane, nextAppearance } from "@/lib/presence"
import { segmentIndexAt, shotNumbers } from "@/lib/segments"
import { PLAYER_SHORTCUTS, useShortcuts } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { type FrameStore, useFrame, useFrameSelector } from "../frame-store"
import { SEGMENT_LABEL } from "../labels"
import { drawSegments, drawSpans, prepare } from "./draw"
import { type Counting, ticks } from "./ticks"
import { type View, centerOn, fit, follow, panBy, tAt, xOf, zoomAt } from "./viewport"

/** Lanes shown under the shots; everyone else is listed in the details panel. */
const MAX_LANES = 5
const ZOOM_STEP = 2

type Axis = {
  view: View
  width: number
  /** The episode's last frame. */
  last: number
  /** Seconds from the first frame to frame `f`, and back. */
  t: (f: number) => number
  frameAt: (t: number) => number
}

/**
 * The episode in time, under the picture: an overview and a ruler, the
 * playhead's row, the shots and the main characters' lanes, all on one axis
 * that zooms (Ctrl/⌘ + wheel or a pinch, + and −, Shift+Z for the whole
 * episode) and scrolls (wheel). The view turns the page as the playhead runs
 * off it. Shots and lanes are drawn on canvases, again only when the view or
 * what they show changes; the playhead is the one part that moves with every
 * frame.
 */
export function Timeline({
  clock,
  fps,
  counting,
  store,
  segments,
  lanes,
  focus,
  inFrame,
  outFrame,
  tc,
  onSeek,
  onScrub,
  onMarkRange,
  onMarkIn,
  onMarkOut,
  onFocus,
  onOpenCharacter,
}: {
  clock: FrameClock
  /** The nominal frame rate, which the timecode counts. */
  fps: number
  /** How frames count on the timecode (drop-frame skips labels). */
  counting: Counting
  store: FrameStore
  segments: Segment[]
  lanes: Lane[]
  focus: string | null
  inFrame: number | null
  outFrame: number | null
  tc: (f: number) => string
  onSeek: (f: number) => void
  /** Scrubbing starts (playback pauses). */
  onScrub: () => void
  /** Frames `start`..`end` (the end not in it) become the clip. */
  onMarkRange: (r: { start: number; end: number }) => void
  onMarkIn: (f: number) => void
  onMarkOut: (f: number) => void
  /** Highlight a character's lane (null: none). */
  onFocus: (characterId: string | null) => void
  onOpenCharacter: (characterId: string) => void
}) {
  const length = clock.rel(clock.frames)
  // Never closer than about a second of frames.
  const minSpan = Math.min(length, clock.duration(0) * 24)
  const [view, setView] = useState<View>(() => fit(length))
  const current = useRef(view)
  useLayoutEffect(() => {
    current.current = view
  })
  const card = useRef<HTMLDivElement>(null)
  const track = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)

  useLayoutEffect(() => {
    const el = track.current
    if (!el) return
    const measure = () => setWidth(el.clientWidth)
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const axis: Axis = useMemo(
    () => ({
      view,
      width,
      last: clock.frames - 1,
      t: (f) => clock.rel(f),
      frameAt: (t) => clock.frameAt(clock.start(0) + t),
    }),
    [view, width, clock],
  )

  // The playhead stays in view: the page turns when it runs off (checked
  // on every frame without drawing anything until it does).
  useEffect(() => {
    let prev = clock.rel(store.get())
    return store.subscribe(() => {
      const t = clock.rel(store.get())
      const next = follow(current.current, t, prev, length)
      prev = t
      if (next !== current.current) {
        current.current = next
        setView(next)
      }
    })
  }, [store, clock, length])

  // Ctrl/⌘ + wheel (and a pinch) zooms where the pointer is; the wheel scrolls.
  useEffect(() => {
    const el = card.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      const r = track.current?.getBoundingClientRect()
      if (!r || r.width === 0) return
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault()
        const at = tAt(current.current, e.clientX - r.left, r.width)
        setView((v) => zoomAt(v, Math.exp(-e.deltaY * 0.0025), at, length, minSpan))
        return
      }
      // Nothing to scroll while the whole episode shows.
      if (current.current.span >= length - 1e-9) return
      e.preventDefault()
      const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY
      setView((v) => panBy(v, (d / r.width) * v.span, length))
    }
    // macOS's WebKit reports a trackpad pinch as gesture events, not as
    // Ctrl + wheel; `scale` grows from 1 over the whole pinch.
    let scale = 1
    const onGestureStart = (e: Event) => {
      e.preventDefault()
      scale = 1
    }
    const onGestureChange = (e: Event) => {
      e.preventDefault()
      const g = e as Event & { scale: number; clientX: number }
      const r = track.current?.getBoundingClientRect()
      if (!r || r.width === 0 || !(g.scale > 0)) return
      const factor = g.scale / scale
      scale = g.scale
      const at = tAt(current.current, g.clientX - r.left, r.width)
      setView((v) => zoomAt(v, factor, at, length, minSpan))
    }
    el.addEventListener("wheel", onWheel, { passive: false })
    el.addEventListener("gesturestart", onGestureStart)
    el.addEventListener("gesturechange", onGestureChange)
    return () => {
      el.removeEventListener("wheel", onWheel)
      el.removeEventListener("gesturestart", onGestureStart)
      el.removeEventListener("gesturechange", onGestureChange)
    }
  }, [length, minSpan])

  const zoom = useCallback(
    (factor: number) =>
      setView((v) => {
        // Around the playhead while it shows, else around the middle.
        const t = clock.rel(store.get())
        const at = t >= v.t0 && t <= v.t0 + v.span ? t : v.t0 + v.span / 2
        return zoomAt(v, factor, at, length, minSpan)
      }),
    [clock, store, length, minSpan],
  )
  useShortcuts(
    PLAYER_SHORTCUTS,
    {
      zoomIn: () => zoom(ZOOM_STEP),
      zoomOut: () => zoom(1 / ZOOM_STEP),
      zoomFit: () => setView(fit(length)),
    },
    { repeat: true },
  )

  const zoomed = view.span < length - 1e-9
  const zoomMenu = (
    <>
      <ContextMenuItem disabled={view.span <= minSpan + 1e-9} onSelect={() => zoom(ZOOM_STEP)} shortcut={["+"]}>
        <MagnifyingGlassPlus /> Zoom in
      </ContextMenuItem>
      <ContextMenuItem disabled={!zoomed} onSelect={() => zoom(1 / ZOOM_STEP)} shortcut={["−"]}>
        <MagnifyingGlassMinus /> Zoom out
      </ContextMenuItem>
      <ContextMenuItem disabled={!zoomed} onSelect={() => setView(fit(length))} shortcut={["Shift", "Z"]}>
        <ArrowsOutLineHorizontal /> The whole episode
      </ContextMenuItem>
    </>
  )
  return (
    <ContextArea menu={zoomMenu}>
      <div
        ref={card}
        className="flex flex-col gap-1.5 rounded-2xl border border-white/[0.07] bg-white/[0.03] px-4 pt-2 pb-3.5 [--lane-label-w:132px]"
      >
        <Row
          label={
            <span className="flex items-center gap-0.5">
              <ZoomButton label="Zoom out" keys={["−"]} onClick={() => zoom(1 / ZOOM_STEP)} disabled={!zoomed}>
                <MagnifyingGlassMinus />
              </ZoomButton>
              <ZoomButton label="Zoom in" keys={["+"]} onClick={() => zoom(ZOOM_STEP)} disabled={view.span <= minSpan + 1e-9}>
                <MagnifyingGlassPlus />
              </ZoomButton>
              <ZoomButton label="The whole episode" keys={["Shift", "Z"]} onClick={() => setView(fit(length))} disabled={!zoomed}>
                <ArrowsOutLineHorizontal />
              </ZoomButton>
            </span>
          }
        >
          <div ref={track} className="relative h-6">
            {zoomed ? (
              <Overview axis={axis} length={length} store={store} inFrame={inFrame} outFrame={outFrame} onView={setView} />
            ) : null}
            <Ruler axis={axis} fps={fps} counting={counting} tc={tc} under={zoomed} />
          </div>
        </Row>
        <Row label="Playhead">
          <Scrub
            axis={axis}
            store={store}
            inFrame={inFrame}
            outFrame={outFrame}
            tc={tc}
            onSeek={onSeek}
            onScrub={onScrub}
            onMarkIn={onMarkIn}
            onMarkOut={onMarkOut}
          />
        </Row>
        {segments.length > 0 && (
          <Row label="Shots">
            <Shots axis={axis} store={store} segments={segments} tc={tc} onSeek={onSeek} onScrub={onScrub} onMarkShot={onMarkRange} />
          </Row>
        )}
        {lanes.length > 0 && (
          <div className="mt-1 flex flex-col gap-1" role="group" aria-label="Characters on screen">
            {lanes.slice(0, MAX_LANES).map((lane) => (
              <LaneRow
                key={lane.character.id}
                axis={axis}
                store={store}
                lane={lane}
                focused={focus === lane.character.id}
                tc={tc}
                onSeek={onSeek}
                onScrub={onScrub}
                onMarkRange={onMarkRange}
                onFocus={onFocus}
                onOpen={onOpenCharacter}
              />
            ))}
          </div>
        )}
      </div>
    </ContextArea>
  )
}

/** A timeline row: its name in the label column, the track beside it. */
function Row({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[var(--lane-label-w)_minmax(0,1fr)] items-center gap-3">
      <span className="min-w-0 text-xs font-medium text-fg-3">{label}</span>
      {children}
    </div>
  )
}

function ZoomButton({
  label,
  keys,
  disabled,
  onClick,
  children,
}: {
  label: string
  keys: string[]
  disabled?: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <Tooltip label={label} shortcut={keys}>
      <button
        onClick={onClick}
        disabled={disabled}
        aria-label={label}
        className="flex size-6 items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-white/[0.08] hover:text-fg-1 disabled:opacity-30 disabled:hover:bg-transparent [&_svg]:size-3.5"
      >
        {children}
      </button>
    </Tooltip>
  )
}

/** Where the playhead's frame is on the axis (CSS pixels from the track's left), or null off view. */
function useHeadX(axis: Axis, store: FrameStore): number | null {
  const frame = useFrame(store)
  const x = xOf(axis.view, axis.t(frame), axis.width)
  return x < -1 || x > axis.width + 1 ? null : x
}

/** A thin line where the playhead is, for the rows under the scrubber. */
function HeadLine({ axis, store }: { axis: Axis; store: FrameStore }) {
  const x = useHeadX(axis, store)
  if (x == null) return null
  return <div className="pointer-events-none absolute inset-y-0 w-px bg-white/70" style={{ left: x }} />
}

/** Round timecodes on the axis; the overview lies over it while zoomed. */
function Ruler({
  axis,
  fps,
  counting,
  tc,
  under,
}: {
  axis: Axis
  fps: number
  counting: Counting
  tc: (f: number) => string
  under: boolean
}) {
  const { frames } = ticks(axis.view, axis.width, fps, axis.frameAt, axis.t, counting)
  return (
    <div className={cn("pointer-events-none absolute inset-x-0 bottom-0 h-3", under && "opacity-60")} aria-hidden>
      {frames.map((f) => (
        <span key={f} className="absolute bottom-0 flex items-end gap-1" style={{ left: xOf(axis.view, axis.t(f), axis.width) }}>
          <span className="h-2 w-px bg-white/25" />
          <span className="timecode -mb-0.5 text-[10px] leading-none text-fg-3">{tc(f)}</span>
        </span>
      ))}
    </div>
  )
}

/**
 * The whole episode as a thin bar while the timeline is zoomed: the part in
 * view as a window to drag, a click elsewhere goes there.
 */
function Overview({
  axis,
  length,
  store,
  inFrame,
  outFrame,
  onView,
}: {
  axis: Axis
  length: number
  store: FrameStore
  inFrame: number | null
  outFrame: number | null
  onView: (v: View | ((v: View) => View)) => void
}) {
  const frame = useFrame(store)
  const drag = useRef<{ x: number; t0: number } | null>(null)
  const pct = (t: number) => (t / Math.max(length, 1e-9)) * 100
  const { view } = axis
  const range = inFrame != null || outFrame != null
  const release = () => (drag.current = null)
  return (
    <div
      className="absolute inset-x-0 top-0 h-2.5 touch-none rounded-full bg-white/[0.06] select-none"
      onPointerDown={(e) => {
        // Only the main button pans: a right-click opens the menu, nothing else.
        if (e.button !== 0) return
        const r = e.currentTarget.getBoundingClientRect()
        const t = ((e.clientX - r.left) / r.width) * length
        if (t >= view.t0 && t <= view.t0 + view.span) {
          e.currentTarget.setPointerCapture(e.pointerId)
          drag.current = { x: e.clientX, t0: view.t0 }
        } else {
          onView((v) => centerOn(v, t, length))
        }
      }}
      onPointerMove={(e) => {
        const d = drag.current
        if (!d) return
        const r = e.currentTarget.getBoundingClientRect()
        const dt = ((e.clientX - d.x) / r.width) * length
        onView((v) => panBy({ ...v, t0: d.t0 }, dt, length))
      }}
      onPointerUp={release}
      // A drag the system took over (a native drag, a touch turned scroll) ends too.
      onPointerCancel={release}
      onLostPointerCapture={release}
      role="scrollbar"
      aria-orientation="horizontal"
      aria-label="Part of the episode in view"
      aria-valuemin={0}
      aria-valuemax={Math.round(length)}
      aria-valuenow={Math.round(view.t0)}
    >
      {range && (
        <div
          className="absolute inset-y-0 rounded-full bg-accent/50"
          style={{
            left: `${pct(axis.t(inFrame ?? 0))}%`,
            width: `${Math.max(0.3, pct(axis.t((outFrame ?? Number.MAX_SAFE_INTEGER) + 1)) - pct(axis.t(inFrame ?? 0)))}%`,
          }}
        />
      )}
      <div
        className="absolute inset-y-0 rounded-full border border-white/40 bg-white/15"
        style={{ left: `${pct(view.t0)}%`, width: `${Math.max(1, pct(view.span))}%` }}
      />
      <div className="absolute inset-y-0 w-px bg-white" style={{ left: `${pct(axis.t(frame))}%` }} />
    </div>
  )
}

/** Hover readout over a track. */
function Tip({ x, children }: { x: number; children: React.ReactNode }) {
  return (
    <div
      className="glass-strong pointer-events-none absolute bottom-full z-10 mb-2 -translate-x-1/2 rounded-lg px-2 py-1 text-xs whitespace-nowrap"
      style={{ left: x }}
    >
      {children}
    </div>
  )
}

/**
 * The playhead's row: drag to scrub (whole frames), the marked range in gold
 * and the timecode under the pointer.
 */
function Scrub({
  axis,
  store,
  inFrame,
  outFrame,
  tc,
  onSeek,
  onScrub,
  onMarkIn,
  onMarkOut,
}: {
  axis: Axis
  store: FrameStore
  inFrame: number | null
  outFrame: number | null
  tc: (f: number) => string
  onSeek: (f: number) => void
  onScrub: () => void
  onMarkIn: (f: number) => void
  onMarkOut: (f: number) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; f: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  // The frame the menu is about: the one right-clicked (the playhead's, from the keyboard).
  const [menuAt, setMenuAt] = useState<number | null>(null)
  const frame = useFrame(store)
  const { view, width } = axis
  const at = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = Math.min(Math.max(clientX - r.left, 0), r.width)
    return { x, f: axis.frameAt(tAt(view, x, r.width)) }
  }
  const x = (f: number) => xOf(view, axis.t(f), width)
  const head = x(frame)
  const { last } = axis
  const range = inFrame != null || outFrame != null
  const from = x(inFrame ?? 0)
  const to = x((outFrame ?? last) + 1)
  const clip = (a: number) => Math.min(Math.max(a, 0), width)
  const menu = menuAt != null && (
    <>
      <ContextMenuLabel className="timecode">{tc(menuAt)}</ContextMenuLabel>
      <ContextMenuItem
        onSelect={() => {
          onScrub()
          onSeek(menuAt)
        }}
      >
        <Crosshair /> Go here
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onMarkIn(menuAt)}>
        <ArrowLineRight /> Mark in here
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onMarkOut(menuAt)}>
        <ArrowLineLeft /> Mark out here
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => void copyText(tc(menuAt), "timecode")}>
        <Copy /> Copy this timecode
      </ContextMenuItem>
    </>
  )
  return (
    <ContextArea menu={menu} onContextMenu={(e) => setMenuAt(e.button === 2 ? at(e.clientX).f : store.get())}>
      <div
        ref={ref}
        role="slider"
        tabIndex={0}
        aria-label="Playhead"
        aria-valuemin={0}
        aria-valuemax={last}
        aria-valuenow={frame}
        aria-valuetext={tc(frame)}
        className="group/scrub relative h-9 touch-none rounded-lg select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-subtle"
        onPointerDown={(e) => {
          // Only the main button seeks: a right-click opens the menu where it is.
          if (e.button !== 0) return
          e.currentTarget.setPointerCapture(e.pointerId)
          setDragging(true)
          onScrub()
          onSeek(at(e.clientX).f)
        }}
        onPointerMove={(e) => {
          const h = at(e.clientX)
          setHover(h)
          if (dragging) onSeek(h.f)
        }}
        onPointerUp={(e) => {
          e.currentTarget.releasePointerCapture(e.pointerId)
          setDragging(false)
        }}
        onPointerCancel={() => setDragging(false)}
        onLostPointerCapture={() => setDragging(false)}
        onPointerLeave={() => !dragging && setHover(null)}
      >
        <div className="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 overflow-hidden rounded-full bg-white/10 transition-[height] duration-[var(--dur-fast)] group-hover/scrub:h-2">
          <div className="absolute inset-y-0 left-0 bg-white/35" style={{ width: clip(head) }} />
          {range && to > 0 && from < width && (
            <div
              className="absolute inset-y-0 bg-accent/60 shadow-[0_0_10px_var(--accent)]"
              style={{ left: clip(from), width: Math.max(2, clip(to) - clip(from)) }}
            />
          )}
        </div>
        {inFrame != null && from >= 0 && from <= width && (
          <div className="absolute top-1.5 bottom-1.5 w-[4px] rounded-l-[3px] border-y-2 border-l-2 border-accent" style={{ left: from }} />
        )}
        {outFrame != null && to >= 0 && to <= width && (
          <div
            className="absolute top-1.5 bottom-1.5 w-[4px] -translate-x-full rounded-r-[3px] border-y-2 border-r-2 border-accent"
            style={{ left: to }}
          />
        )}
        {head >= -8 && head <= width + 8 && (
          <div className="pointer-events-none absolute top-0 bottom-0 -translate-x-1/2" style={{ left: head }}>
            <div className="mx-auto h-full w-[2px] rounded-full bg-white shadow-[0_0_8px_rgb(255_255_255/0.6)]" />
            <div className="absolute top-1/2 left-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_0_0_3px_rgb(0_0_0/0.35),0_0_12px_rgb(255_255_255/0.5)]" />
          </div>
        )}
        {hover && (
          <Tip x={hover.x}>
            <span className="timecode text-fg-1">{tc(hover.f)}</span>
          </Tip>
        )}
      </div>
    </ContextArea>
  )
}

/**
 * A canvas filling its track, drawn again when `draw` (its data), the axis
 * or the screen's pixel density (another monitor, the webview zoomed) change.
 */
function TrackCanvas({ axis, height, draw }: { axis: Axis; height: number; draw: (ctx: CanvasRenderingContext2D, width: number) => void }) {
  const ref = useRef<HTMLCanvasElement>(null)
  const dpr = usePixelRatio()
  useLayoutEffect(() => {
    const c = ref.current
    if (!c || axis.width === 0) return
    const ctx = prepare(c, axis.width, height)
    if (ctx) draw(ctx, axis.width)
  }, [axis, height, draw, dpr])
  return <canvas ref={ref} aria-hidden className="absolute inset-0 size-full" />
}

/** `devicePixelRatio`, as it changes. */
function usePixelRatio(): number {
  const [dpr, setDpr] = useState(() => window.devicePixelRatio || 1)
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return
    // Matches until the ratio moves away from this one.
    const q = window.matchMedia(`(resolution: ${dpr}dppx)`)
    const onChange = () => setDpr(window.devicePixelRatio || 1)
    q.addEventListener("change", onChange)
    return () => q.removeEventListener("change", onChange)
  }, [dpr])
  return dpr
}

/**
 * The shots on the axis; the shot under the playhead outlined (drawn again
 * only when it changes). Click a shot to go to its first frame;
 * double-click marks it as the clip.
 */
function Shots({
  axis,
  store,
  segments,
  tc,
  onSeek,
  onScrub,
  onMarkShot,
}: {
  axis: Axis
  store: FrameStore
  segments: Segment[]
  tc: (f: number) => string
  onSeek: (f: number) => void
  onScrub: () => void
  onMarkShot: (s: Segment) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; i: number } | null>(null)
  // The shot the menu is about: the one right-clicked.
  const [menuShot, setMenuShot] = useState(-1)
  // The shot a click was on: the page may turn under the pointer before the
  // second click of a double-click (going to a shot that starts off view).
  const clicked = useRef(-1)
  const numbers = useMemo(() => shotNumbers(segments), [segments])
  const current = useFrameSelector(
    store,
    useCallback((f: number) => segmentIndexAt(segments, f), [segments]),
  )
  const { view, width } = axis
  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, w: number) => drawSegments(ctx, w, 20, view, segments, numbers, axis.t),
    [view, segments, numbers, axis.t],
  )
  const indexAt = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = Math.min(Math.max(clientX - r.left, 0), r.width)
    return { x, i: segmentIndexAt(segments, axis.frameAt(tAt(view, x, r.width))) }
  }
  const cur = current >= 0 ? segments[current] : null
  const hovered = hover && hover.i >= 0 ? segments[hover.i] : null
  const seconds = (s: Segment) => (axis.t(s.end) - axis.t(s.start)).toFixed(1)
  const name = (i: number) => (segments[i].kind === "shot" ? `Shot ${numbers[i] + 1}` : SEGMENT_LABEL[segments[i].kind])
  const ms = menuShot >= 0 ? segments[menuShot] : undefined
  const menu = ms && (
    <>
      <ContextMenuLabel>
        {name(menuShot)} · <span className="timecode">{tc(ms.start)}</span>
      </ContextMenuLabel>
      <ContextMenuItem
        onSelect={() => {
          onScrub()
          onSeek(ms.start)
        }}
      >
        <Crosshair /> {ms.kind === "shot" ? "Go to the shot" : "Go there"}
      </ContextMenuItem>
      {ms.kind === "shot" && (
        <ContextMenuItem onSelect={() => onMarkShot(ms)}>
          <FilmStrip /> Mark it as the clip
        </ContextMenuItem>
      )}
      <ContextMenuItem onSelect={() => void copyText(`${tc(ms.start)} - ${tc(ms.end - 1)}`, "timecodes")}>
        <Copy /> Copy its timecodes
      </ContextMenuItem>
    </>
  )
  return (
    <ContextArea menu={menu} onContextMenu={(e) => setMenuShot(indexAt(e.clientX).i)}>
      <div
        ref={ref}
        className="relative h-5 rounded-md select-none"
        role="group"
        aria-label="Shots"
        onPointerMove={(e) => setHover(indexAt(e.clientX))}
        onPointerLeave={() => setHover(null)}
        onClick={(e) => {
          const i = e.detail > 1 && clicked.current >= 0 ? clicked.current : indexAt(e.clientX).i
          clicked.current = i
          if (i < 0) return
          onScrub()
          onSeek(segments[i].start)
        }}
        onDoubleClick={(e) => {
          const i = clicked.current >= 0 ? clicked.current : indexAt(e.clientX).i
          if (segments[i]?.kind === "shot") onMarkShot(segments[i])
        }}
      >
        <TrackCanvas axis={axis} height={20} draw={draw} />
        {cur && <Outline left={xOf(view, axis.t(cur.start), width)} right={xOf(view, axis.t(cur.end), width)} width={width} />}
        <HeadLine axis={axis} store={store} />
        {hover && hovered && (
          <Tip x={hover.x}>
            <span className="text-fg-1">{name(hover.i)}</span>
            <span className="timecode text-fg-3">
              {" "}
              · {tc(hovered.start)} · {seconds(hovered)} s{hovered.tiny ? " · tiny" : ""}
            </span>
          </Tip>
        )}
      </div>
    </ContextArea>
  )
}

/** The shot under the playhead, cut to the part in view. */
function Outline({ left, right, width }: { left: number; right: number; width: number }) {
  if (right < 0 || left > width) return null
  const a = Math.max(0, left)
  const b = Math.min(width, right)
  return (
    <div
      className="pointer-events-none absolute inset-y-0 rounded-[3px] bg-accent/25 ring-1 ring-accent ring-inset"
      style={{ left: a, width: Math.max(2, b - a) }}
    />
  )
}

/**
 * One main character's time on screen. Click a span to go there; the
 * focused character is gold. The menu speaks of the appearance right-clicked
 * and of the character.
 */
function LaneRow({
  axis,
  store,
  lane,
  focused,
  tc,
  onSeek,
  onScrub,
  onMarkRange,
  onFocus,
  onOpen,
}: {
  axis: Axis
  store: FrameStore
  lane: Lane
  focused: boolean
  tc: (f: number) => string
  onSeek: (f: number) => void
  onScrub: () => void
  onMarkRange: (r: { start: number; end: number }) => void
  onFocus: (characterId: string | null) => void
  onOpen: (characterId: string) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [hover, setHover] = useState<{ x: number; span: Presence } | null>(null)
  const [menuSpan, setMenuSpan] = useState<Presence | null>(null)
  const c = lane.character
  const { view } = axis
  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, w: number) => drawSpans(ctx, w, 24, view, lane.spans, focused, axis.t),
    [view, lane.spans, focused, axis.t],
  )
  const spanAt = (clientX: number) => {
    const r = ref.current!.getBoundingClientRect()
    const x = clientX - r.left
    // The span under the pointer, else the nearest within a few pixels.
    let best: Presence | null = null
    let dist = 6
    for (const s of lane.spans) {
      const a = xOf(view, axis.t(s.start), r.width)
      const b = xOf(view, axis.t(s.end), r.width)
      const d = x < a ? a - x : x > b ? x - b : 0
      if (d <= dist) {
        dist = d
        best = s
      }
    }
    return best ? { x, span: best } : null
  }
  const go = (f: number) => {
    onScrub()
    onSeek(f)
  }
  const menu = (
    <>
      <ContextMenuLabel>
        {c.label}
        {menuSpan && (
          <>
            {" "}
            · <span className="timecode">{tc(menuSpan.start)}</span>
            {menuSpan.body ? " · no face" : ""}
          </>
        )}
      </ContextMenuLabel>
      {menuSpan && (
        <>
          <ContextMenuItem onSelect={() => go(menuSpan.start)}>
            <Crosshair /> Go to this appearance
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => onMarkRange(menuSpan)}>
            <FilmStrip /> Mark it as the clip
          </ContextMenuItem>
        </>
      )}
      <ContextMenuItem
        onSelect={() => {
          const f = nextAppearance(lane, store.get())
          if (f != null) go(f)
        }}
      >
        <SkipForward /> Next appearance
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => onFocus(focused ? null : c.id)}>
        <UserFocus /> {focused ? "Stop highlighting" : "Highlight"}
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onOpen(c.id)}>
        <User /> Open in the cast
      </ContextMenuItem>
    </>
  )
  return (
    <ContextArea
      menu={menu}
      // On the track the appearance under the pointer; on the name, none.
      onContextMenu={(e) => setMenuSpan(ref.current?.contains(e.target as Node) ? (spanAt(e.clientX)?.span ?? null) : null)}
    >
      <div className="group grid grid-cols-[var(--lane-label-w)_minmax(0,1fr)] items-center gap-3">
        <span className="flex min-w-0 items-center gap-2">
          {c.coverSmall && <img src={assetUrl(c.coverSmall)} alt="" className="size-5 shrink-0 rounded-full object-cover" />}
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
            if (!h) return
            onScrub()
            onSeek(h.span.start)
          }}
        >
          <TrackCanvas axis={axis} height={24} draw={draw} />
          <HeadLine axis={axis} store={store} />
          {hover && (
            <Tip x={hover.x}>
              <span className="text-fg-1">{c.label}</span>
              <span className="timecode text-fg-3">
                {" "}
                · {tc(hover.span.start)}
                {hover.span.body ? " · no face" : ""}
              </span>
            </Tip>
          )}
        </div>
      </div>
    </ContextArea>
  )
}
