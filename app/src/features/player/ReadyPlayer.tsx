import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"
import type { FrameMap, MediaItem, Segment } from "@/bindings"
import { ExportDialog, type ExportClip } from "@/features/export/ExportDialog"
import { COPY_TIMECODES_REQUEST, EXPORT_REQUEST, REVEAL_REQUEST } from "@/lib/actions"
import { FrameClock } from "@/lib/frame-clock"
import { formatDuration } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { lanesOf, useCharacterMap } from "@/lib/presence"
import { useCast, usePresence, useSegments } from "@/lib/queries"
import { nextShotStart, prevShotStart, shotAt } from "@/lib/segments"
import { PLAYER_SHORTCUTS, useShortcuts } from "@/lib/shortcuts"
import { parseTimecode, timecode } from "@/lib/timecode"
import { NO_MARKS, useMarks } from "@/state/marks"
import { usePlayback } from "@/state/playback"
import { Ambilight } from "./Ambilight"
import { CharactersPanel } from "./panels/CharactersPanel"
import { ClipCard } from "./panels/ClipCard"
import { KeysCard } from "./panels/KeysCard"
import { ShotsPanel } from "./panels/ShotsPanel"
import { SourceCard } from "./panels/SourceCard"
import type { ShuttleKey } from "./shuttle"
import { Timeline } from "./timeline/Timeline"
import { Transport } from "./Transport"
import { usePlayer } from "./use-player"

const NO_SEGMENTS: Segment[] = []

/**
 * A playable episode: the picture in its dark room, the transport, the
 * timeline and the details. It doesn't draw again with every video frame:
 * what shows the frame follows it on its own (see `FrameStore`), and
 * everything here reads it when it acts.
 */
export function ReadyPlayer({
  pid,
  media: m,
  map,
  openAt,
  inspector,
}: {
  pid: string
  media: MediaItem
  map: FrameMap
  openAt?: number
  /** The details panel is open. */
  inspector: boolean
}) {
  const clock = useMemo(() => new FrameClock(map), [map])
  const { videoRef, store, rate, seek, step, toggle, pause, shuttle } = usePlayer(clock)
  // Opened at a frame (a face in the cast, a shot of a pack): go there once.
  const opened = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (openAt != null && opened.current !== openAt) {
      opened.current = openAt
      seek(openAt)
    }
  }, [openAt, seek])
  const { data: cast } = useCast(pid)
  const characters = useCharacterMap(cast?.characters)
  const { data: presence } = usePresence(pid, m.id, m.faces === "ready")
  const lanes = useMemo(() => lanesOf(presence ?? [], characters), [presence, characters])
  const [focus, setFocus] = useState<string | null>(null)
  const marks = useMarks((s) => s.byMedia[m.id] ?? NO_MARKS)
  const setMarks = useMarks((s) => s.set)
  const inFrame = marks.in
  const outFrame = marks.out
  const [exportOpen, setExportOpen] = useState(false)
  const [gotoOpen, setGotoOpen] = useState(false)
  const { volume, muted, setVolume, toggleMute } = usePlayback()
  const { data: segments } = useSegments(pid, m.id, m.shots === "ready")
  const shots = m.shots === "ready" ? (segments ?? NO_SEGMENTS) : NO_SEGMENTS
  const fps = map.fps
  const tc = useCallback((f: number) => timecode(f, fps, map.dropFrame), [fps, map.dropFrame])
  const parse = useCallback((text: string, from: number) => parseTimecode(text, fps, map.dropFrame, from), [fps, map.dropFrame])
  const last = clock.frames - 1
  const oneSecond = Math.max(1, Math.round(fps.num / fps.den))

  const markIn = (f = store.get()) => setMarks(m.id, { in: f, ...(outFrame != null && outFrame < f ? { out: null } : {}) })
  const markOut = (f = store.get()) => setMarks(m.id, { out: f, ...(inFrame != null && inFrame > f ? { in: null } : {}) })
  const clear = () => setMarks(m.id, { in: null, out: null })
  /** The whole shot (its clean range, without transition frames) becomes the clip. */
  const markShot = (s: Segment | null = shotAt(shots, store.get())) => {
    if (s) setMarks(m.id, { in: s.start, out: s.end - 1 })
  }
  const goShot = (to: number | null) => {
    if (to != null) seek(to)
  }
  const hasMarks = inFrame != null || outFrame != null
  const start = inFrame ?? 0
  const end = (outFrame ?? last) + 1
  const length = end - start

  const seconds = clock.rel(end) - clock.rel(start)
  const clip: ExportClip = {
    mediaId: m.id,
    start,
    end,
    name: null,
    summary: `${m.label ? `${m.label} · ` : ""}${tc(start)} – ${tc(end - 1)} · ${formatDuration(seconds)} (${length} frames)`,
  }
  // One mark alone runs the clip to the episode's edge: say so before an
  // export of most of an episode starts.
  const oneSided =
    inFrame != null && outFrame == null
      ? "Only an in point is marked, so the clip runs to the end of the episode. To end it sooner, close this and press O where it should stop."
      : outFrame != null && inFrame == null
        ? "Only an out point is marked, so the clip starts at the beginning of the episode. To start it later, close this and press I where it should begin."
        : undefined
  const canExport = hasMarks && length > 0
  const openExport = () => {
    pause()
    setExportOpen(true)
  }

  useEffect(() => {
    const onRequest = () => {
      if (!canExport) return
      pause()
      setExportOpen(true)
    }
    window.addEventListener(EXPORT_REQUEST, onRequest)
    return () => window.removeEventListener(EXPORT_REQUEST, onRequest)
  }, [canExport, pause])

  // macOS menu: File → Reveal, Edit → Copy Timecodes.
  useEffect(() => {
    const reveal = () => void commands.revealPath(m.path).catch((e) => toast.error(errorMessage(e)))
    const copy = () => {
      const text = hasMarks ? `${tc(start)} - ${tc(end - 1)}` : tc(store.get())
      commands
        .clipboardWrite(text)
        .then(() => toast("Copied timecode", { description: text }))
        .catch((e) => toast.error("Couldn't copy the timecode", { description: errorMessage(e) }))
    }
    window.addEventListener(REVEAL_REQUEST, reveal)
    window.addEventListener(COPY_TIMECODES_REQUEST, copy)
    return () => {
      window.removeEventListener(REVEAL_REQUEST, reveal)
      window.removeEventListener(COPY_TIMECODES_REQUEST, copy)
    }
  })

  // While the player is open, clicked buttons don't take focus: otherwise
  // Space would press the last clicked button again (WebView2 even marks it
  // :focus-visible on the first key) instead of play/pause. Menus, lists and
  // dialogs keep normal focus handling; Tab still reaches every button.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const b = e.target instanceof Element ? e.target.closest("button") : null
      if (b && !b.closest("[role=dialog], [role=menu], [role=listbox], [role=combobox], [aria-haspopup]")) {
        e.preventDefault()
      }
    }
    window.addEventListener("mousedown", onDown, true)
    return () => window.removeEventListener("mousedown", onDown, true)
  }, [])

  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    v.volume = volume
    v.muted = muted
  }, [videoRef, volume, muted])

  // K held down: J and L step single frames instead of shuttling.
  const kHeld = useRef(false)
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k") kHeld.current = true
    }
    const up = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k") kHeld.current = false
    }
    const reset = () => (kHeld.current = false)
    window.addEventListener("keydown", down)
    window.addEventListener("keyup", up)
    window.addEventListener("blur", reset)
    return () => {
      window.removeEventListener("keydown", down)
      window.removeEventListener("keyup", up)
      window.removeEventListener("blur", reset)
    }
  }, [])

  // The player's keys (PLAYER_SHORTCUTS). A held arrow keeps stepping; a
  // held key that toggles or marks acts once.
  const once =
    (run: (e: KeyboardEvent) => unknown) =>
    (e: KeyboardEvent): void => {
      if (!e.repeat) run(e)
    }
  useShortcuts(
    PLAYER_SHORTCUTS,
    {
      playPause: once(() => toggle()),
      shuttle: (e) => {
        const k = e.key.toLowerCase() as ShuttleKey
        if (k !== "k" && kHeld.current) return step(k === "j" ? -1 : 1)
        if (e.repeat) return
        shuttle(k)
      },
      frame: (e) => step(e.key === "ArrowLeft" ? -1 : 1),
      second: (e) => step(e.key === "ArrowLeft" ? -oneSecond : oneSecond),
      shot: (e) => {
        if (shots.length === 0) return false
        const f = store.get()
        goShot(e.key === "ArrowUp" ? prevShotStart(shots, f) : nextShotStart(shots, f))
      },
      ends: (e) => seek(e.key === "Home" ? 0 : last),
      mark: once((e) => (e.key.toLowerCase() === "i" ? markIn() : markOut())),
      goToMark: (e) => {
        const to = e.key.toLowerCase() === "i" ? inFrame : outFrame
        if (to != null) seek(to)
      },
      markShot: once(() => markShot()),
      clearMarks: once(() => clear()),
      goTo: once(() => {
        pause()
        setGotoOpen(true)
      }),
      mute: once(() => toggleMute()),
      export: once(() => {
        if (canExport) openExport()
      }),
    },
    { repeat: true },
  )

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden px-10 pt-8 pb-5" onClick={toggle}>
          <Ambilight video={videoRef} store={store} />
          {/* The glow fades out before the controls. */}
          <div aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-24 bg-gradient-to-t from-inset to-transparent" />
          <video
            ref={videoRef}
            src={assetUrl(map.src)}
            preload="auto"
            // The asset protocol answers with CORS headers; keeps frames readable for canvas grabs.
            crossOrigin="anonymous"
            playsInline
            disablePictureInPicture
            className="relative max-h-full max-w-full rounded-2xl bg-black shadow-[0_50px_120px_-40px_rgb(0_0_0/1),0_0_0_1px_rgb(255_255_255/0.07)]"
          />
        </div>

        {/* Secondary readouts fold away as the player column narrows. */}
        <div className="@container relative shrink-0 px-6 pb-5">
          <Transport
            store={store}
            last={last}
            rate={rate}
            tc={tc}
            parse={parse}
            goto={gotoOpen}
            onGoto={(open) => {
              if (open) pause()
              setGotoOpen(open)
            }}
            onSeek={seek}
            onStep={step}
            onToggle={toggle}
            inFrame={inFrame}
            outFrame={outFrame}
            onMarkIn={() => markIn()}
            onMarkOut={() => markOut()}
            showExport={!inspector}
            canExport={canExport}
            onExport={openExport}
            volume={volume}
            muted={muted}
            onVolume={setVolume}
            onToggleMute={toggleMute}
          />
          <Timeline
            clock={clock}
            fps={fps.num / fps.den}
            store={store}
            segments={shots}
            lanes={lanes}
            focus={focus}
            inFrame={inFrame}
            outFrame={outFrame}
            tc={tc}
            onSeek={seek}
            onScrub={pause}
            onMarkShot={markShot}
          />
        </div>
      </div>

      {inspector && (
        <aside className="flex w-[340px] shrink-0 flex-col gap-3 overflow-y-auto border-l border-white/[0.06] bg-canvas p-3">
          <ClipCard
            inFrame={inFrame}
            outFrame={outFrame}
            length={length}
            tc={tc}
            canExport={canExport}
            onMarkIn={() => markIn()}
            onMarkOut={() => markOut()}
            onGo={seek}
            onClear={clear}
            onExport={openExport}
          />
          <ShotsPanel pid={pid} media={m} segments={shots} store={store} tc={tc} onMark={() => markShot()} onGo={seek} />
          <CharactersPanel
            pid={pid}
            media={m}
            lanes={lanes}
            segments={shots}
            store={store}
            focus={focus}
            onFocus={setFocus}
            onGo={(f) => {
              pause()
              seek(f)
            }}
          />
          <SourceCard pid={pid} media={m} fps={fps} />
          <KeysCard />
        </aside>
      )}

      <ExportDialog projectId={pid} clips={[clip]} notice={oneSided} open={exportOpen} onOpenChange={setExportOpen} />
    </div>
  )
}
