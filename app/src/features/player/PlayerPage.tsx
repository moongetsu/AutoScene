import type * as React from "react"
import { useEffect, useMemo, useRef, useState } from "react"
import { Link, useParams, useSearch } from "@tanstack/react-router"
import {
  ArrowCounterClockwise,
  ArrowLeft,
  CaretLeft,
  CaretLineLeft,
  CaretLineRight,
  CaretRight,
  CircleNotch,
  FolderOpen,
  Pause,
  Play,
  Scissors,
  SkipBack,
  SidebarSimple,
  SkipForward,
  SpeakerHigh,
  SpeakerLow,
  SpeakerSlash,
  Users,
  X,
} from "@/components/icons"
import { toast } from "sonner"
import type { FrameMap, MediaItem, Segment } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Progress } from "@/components/ui/progress"
import { Select } from "@/components/ui/select"
import { TopBar } from "@/components/ui/topbar"
import { Tooltip } from "@/components/ui/tooltip"
import { ExportDialog, type ExportClip } from "@/features/export/ExportDialog"
import { COPY_TIMECODES_REQUEST, EXPORT_REQUEST, REVEAL_REQUEST } from "@/lib/actions"
import { FrameClock } from "@/lib/frame-clock"
import { formatDuration, formatPercent } from "@/lib/format"
import { commands, errorMessage, isAppError } from "@/lib/ipc"
import {
  assetUrl,
  audioOption,
  codecLabel,
  currentAudio,
  fileStem,
  formatFlags,
  mediaTitle,
  releaseTitle,
} from "@/lib/media"
import { modKey } from "@/lib/platform"
import { PLAYER_SHORTCUTS } from "@/lib/shortcuts"
import { useCast, useFrameMap, useMediaList, usePresence, useProject, useSegments } from "@/lib/queries"
import { type Lane, charactersIn, lanesOf, useCharacterMap } from "@/lib/presence"
import { nextShotStart, prevShotStart, segmentIndexAt, shotAt, shotNumbers } from "@/lib/segments"
import { formatFps, parseTimecode, timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"
import { NO_MARKS, useMarks } from "@/state/marks"
import { usePlayback } from "@/state/playback"
import { useUi } from "@/state/ui"
import { CharacterLanes } from "./CharacterLanes"
import { Scrubber } from "./Scrubber"
import { ShotStrip } from "./ShotStrip"
import { usePlayer } from "./use-player"

/** Same shape as the backend's default clip names: `Title_S01E03_00-12-03-05`. */
function suggestedName(title: string, m: MediaItem, tc: string): string {
  const what = m.label || fileStem(m.fileName)
  return `${title} ${what} ${tc.replace(/[:;]/g, "-")}`
    .replace(/[<>:"/\\|?*]+/g, " ")
    .trim()
    .replace(/\s+/g, "_")
}

export function PlayerPage() {
  const { pid, mid } = useParams({ from: "/p/$pid/timeline/$mid" })
  const { f: openAt } = useSearch({ from: "/p/$pid/timeline/$mid" })
  const mediaQuery = useMediaList(pid)
  const { data: media } = mediaQuery
  const { data: project } = useProject(pid)
  const m = media?.find((x) => x.id === mid)
  const ready = m?.status === "ready"
  const map = useFrameMap(pid, mid, ready)
  const index = media?.findIndex((x) => x.id === mid) ?? -1
  const prev = index > 0 ? media![index - 1] : undefined
  const next = index >= 0 && index < (media?.length ?? 0) - 1 ? media![index + 1] : undefined
  const inspector = useUi((s) => s.inspectorOpen)
  const toggleInspector = useUi((s) => s.toggleInspector)

  return (
    <div className="flex h-full min-h-0 flex-col bg-inset">
      <TopBar
        back={{ label: project?.kind === "movie" ? "Files" : "Episodes", to: `/p/${pid}/episodes` }}
        title={
          <span className="flex min-w-0 items-baseline gap-2.5" title={m?.fileName}>
            {m?.label && <span className="timecode shrink-0 text-md font-medium text-fg-2">{m.label}</span>}
            <span className="truncate">{m ? releaseTitle(m.fileName) : ""}</span>
          </span>
        }
        className="border-white/[0.05] bg-black/50"
      >
        {map.data?.isProxy && (
          <Tooltip label="Playing a preview copy; exports always cut from the original file.">
            <span>
              <Badge tone="outline">Preview</Badge>
            </span>
          </Tooltip>
        )}
        <div className="flex items-center gap-1">
          <EpisodeNav pid={pid} target={prev} dir="prev" />
          <EpisodeNav pid={pid} target={next} dir="next" />
        </div>
        <Tooltip label={inspector ? "Hide details" : "Show details"}>
          <Button
            size="icon-sm"
            variant={inspector ? "secondary" : "ghost"}
            aria-pressed={inspector}
            aria-label="Details"
            onClick={toggleInspector}
          >
            <SidebarSimple mirrored />
          </Button>
        </Tooltip>
      </TopBar>
      {m && ready && map.data ? (
        <ReadyPlayer
          key={`${mid}:${map.data.src}`}
          pid={pid}
          media={m}
          map={map.data}
          openAt={openAt}
          inspector={inspector}
        />
      ) : (
        <NotReady
          pid={pid}
          media={m}
          error={map.error}
          gone={mediaQuery.isSuccess && !m}
          listError={mediaQuery.error}
          onRetryList={() => void mediaQuery.refetch()}
        />
      )}
    </div>
  )
}

function EpisodeNav({ pid, target, dir }: { pid: string; target?: MediaItem; dir: "prev" | "next" }) {
  const label = dir === "prev" ? "Previous episode" : "Next episode"
  const icon = dir === "prev" ? <CaretLeft /> : <CaretRight />
  if (!target) {
    return (
      <Button size="icon-sm" variant="ghost" disabled aria-label={label}>
        {icon}
      </Button>
    )
  }
  return (
    <Tooltip label={`${label}: ${mediaTitle(target)}`}>
      <Button size="icon-sm" variant="secondary" asChild>
        <Link to="/p/$pid/timeline/$mid" params={{ pid, mid: target.id }} aria-label={label}>
          {icon}
        </Link>
      </Button>
    </Tooltip>
  )
}

/** Poster with what's happening instead of a player. */
function NotReady({
  pid,
  media: m,
  error,
  gone,
  listError,
  onRetryList,
}: {
  pid: string
  media?: MediaItem
  error: unknown
  /** The project's episodes loaded, and this one isn't among them. */
  gone: boolean
  /** The episodes couldn't be loaded. */
  listError: unknown
  onRetryList: () => void
}) {
  const job = useMediaJob(m?.id)
  const needsPreview = m?.status === "ready" && isAppError(error) && error.code === "not_ready"
  let title = "Loading…"
  let body: React.ReactNode = null
  if (listError) {
    title = "This episode couldn't be loaded"
    body = errorMessage(listError)
  } else if (gone) {
    title = "This episode isn't in the project any more"
    body = "It may have been removed from the project."
  } else if (m?.status === "queued" || m?.status === "indexing") {
    title = "Indexing"
    body = "Every frame is being indexed so cuts land exactly. Playback starts when it's done."
  } else if (m?.status === "failed") {
    title = "This file couldn't be indexed"
    body = m.error
  } else if (m?.status === "missing") {
    title = "File not found"
    body = m.path
  } else if (needsPreview) {
    title = job ? "Preparing preview" : m?.proxy === "failed" ? "Preview failed" : "Preview not prepared"
    body = job
      ? "This format needs a lightweight preview copy for frame-accurate playback. Exports still cut from the original."
      : error.message
  } else if (error) {
    title = "Can't play this file"
    body = errorMessage(error)
  }
  const retryable = m && (m.status === "failed" || m.status === "missing")
  return (
    <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
      {m?.poster && (
        <img
          src={assetUrl(m.posterSmall ?? m.poster)}
          alt=""
          className="absolute inset-0 size-full scale-110 object-cover opacity-30 blur-2xl"
        />
      )}
      <div className="glass-strong relative flex w-[min(460px,90%)] flex-col items-center rounded-3xl px-8 py-8 text-center">
        {job ? <CircleNotch className="size-6 animate-spin text-accent" /> : null}
        <h2 className="display mt-3 text-3xl text-fg-1">{title}</h2>
        {body && <p className="mt-2 text-md break-words text-fg-2">{body}</p>}
        {job && (
          <div className="mt-6 w-full">
            <Progress value={job.state === "running" ? job.progress : null} />
            <div className="mt-2 flex justify-between text-xs text-fg-3">
              <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "")}</span>
              <span className="timecode">{formatPercent(job.progress)}</span>
            </div>
          </div>
        )}
        {!job && needsPreview && (
          <Button
            className="mt-6"
            variant="primary"
            onClick={() => void commands.mediaPreparePreview(pid, m!.id).catch((e) => toast.error(errorMessage(e)))}
          >
            <ArrowCounterClockwise /> Prepare preview
          </Button>
        )}
        {listError ? (
          <Button className="mt-6" variant="primary" onClick={onRetryList}>
            <ArrowCounterClockwise /> Try again
          </Button>
        ) : null}
        {gone && (
          <Button className="mt-6" variant="primary" asChild>
            <Link to="/p/$pid/episodes" params={{ pid }}>
              <ArrowLeft /> Back to the episodes
            </Link>
          </Button>
        )}
        {!job && retryable && (
          <Button
            className="mt-6"
            variant="primary"
            onClick={() => void commands.mediaRetry(pid, m.id).catch((e) => toast.error(errorMessage(e)))}
          >
            <ArrowCounterClockwise /> Try again
          </Button>
        )}
      </div>
    </div>
  )
}

function ReadyPlayer({
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
  const { videoRef, frame, playing, seek, step, toggle, pause } = usePlayer(clock)
  // Opened at a frame (a face in the cast): go there once.
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
  const gotoRef = useRef<HTMLInputElement | null>(null)
  const [gotoOpen, setGotoOpen] = useState(false)
  const { volume, muted, setVolume, toggleMute } = usePlayback()
  const { data: project } = useProject(pid)
  const { data: segments } = useSegments(pid, m.id, m.shots === "ready")
  const shots = m.shots === "ready" ? (segments ?? []) : []
  const fps = map.fps
  const tc = (f: number) => timecode(f, fps, map.dropFrame)
  const last = clock.frames - 1
  const oneSecond = Math.max(1, Math.round(fps.num / fps.den))

  const markIn = (f = frame) => setMarks(m.id, { in: f, ...(outFrame != null && outFrame < f ? { out: null } : {}) })
  const markOut = (f = frame) => setMarks(m.id, { out: f, ...(inFrame != null && inFrame > f ? { in: null } : {}) })
  const clear = () => setMarks(m.id, { in: null, out: null })
  /** The whole shot (its clean range, without transition frames) becomes the clip. */
  const markShot = (s: Segment | null = shotAt(shots, frame)) => {
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
    suggestedName: suggestedName(project?.title ?? "", m, tc(start)),
  }
  // One mark alone runs the clip to the episode's edge: say so before an
  // export of most of an episode starts.
  const oneSided =
    inFrame != null && outFrame == null
      ? "Only an in point is marked, so the clip runs to the end of the episode. Press O to mark an out point."
      : outFrame != null && inFrame == null
        ? "Only an out point is marked, so the clip starts at the beginning of the episode. Press I to mark an in point."
        : undefined
  const canExport = hasMarks && length > 0

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
      const text = hasMarks ? `${tc(start)} - ${tc(end - 1)}` : tc(frame)
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

  // Select the go-to field once when it opens, not on every re-render
  // (that would wipe what is being typed).
  useEffect(() => {
    if (!gotoOpen) return
    gotoRef.current?.focus()
    gotoRef.current?.select()
  }, [gotoOpen])

  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    v.volume = volume
    v.muted = muted
  }, [videoRef, volume, muted])

  // Keyboard: the player owns these keys while it's on screen.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target instanceof Element ? e.target : null
      if (exportOpen || t?.closest("input, textarea, select, [role=dialog], [role=menu], [role=listbox]")) return
      // A button or link reached with Tab answers Space and Enter itself
      // (clicked ones don't keep focus, see below).
      if ((e.key === " " || e.key === "Enter") && t?.closest("button, a, [role=button]")) return
      const mod = modKey === "⌘" ? e.metaKey : e.ctrlKey
      const k = e.key.toLowerCase()
      let handled = true
      if (mod && k === "e") {
        if (canExport) {
          pause()
          setExportOpen(true)
        }
      } else if (mod && k === "g") {
        pause()
        setGotoOpen(true)
      } else if (mod) handled = false
      else if (k === " " || k === "k") toggle()
      else if (e.key === "ArrowLeft") step(e.shiftKey ? -oneSecond : -1)
      else if (e.key === "ArrowRight") step(e.shiftKey ? oneSecond : 1)
      else if (e.key === "ArrowUp" && shots.length > 0) goShot(prevShotStart(shots, frame))
      else if (e.key === "ArrowDown" && shots.length > 0) goShot(nextShotStart(shots, frame))
      else if (e.key === "Home") seek(0)
      else if (e.key === "End") seek(last)
      else if (k === "i" && e.shiftKey) {
        if (inFrame != null) seek(inFrame)
      } else if (k === "o" && e.shiftKey) {
        if (outFrame != null) seek(outFrame)
      } else if (k === "i") markIn()
      else if (k === "o") markOut()
      else if (k === "x" && e.altKey) clear()
      else if (k === "x") markShot()
      else if (k === "m") toggleMute()
      else handled = false
      if (handled) e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  })

  const audio = m.info?.audio ?? []
  const openExport = () => {
    pause()
    setExportOpen(true)
  }

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <div
          className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden px-10 pt-8 pb-5"
          onClick={toggle}
        >
          <Ambilight video={videoRef} frame={frame} />
          {/* The glow fades out before the controls. */}
          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-0 bottom-0 h-24 bg-gradient-to-t from-inset to-transparent"
          />
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
          <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 pb-4">
            <div className="flex min-w-0 items-baseline gap-3">
              {gotoOpen ? (
                <input
                  ref={gotoRef}
                  aria-label="Go to timecode or frame"
                  defaultValue={tc(frame)}
                  spellCheck={false}
                  className="timecode h-10 w-[150px] rounded-xl border border-accent-line bg-white/[0.06] px-3 text-lg text-fg-1 outline-none @3xl:w-[190px] @3xl:text-2xl"
                  onBlur={() => setGotoOpen(false)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setGotoOpen(false)
                    if (e.key !== "Enter") return
                    const f = parseTimecode(e.currentTarget.value, fps, map.dropFrame, frame)
                    if (f == null) {
                      toast.error("That isn't a timecode or frame number", {
                        description: "Try 00:12:03:05, a frame like 17099, or +48 to jump ahead.",
                      })
                      return
                    }
                    seek(f)
                    setGotoOpen(false)
                  }}
                />
              ) : (
                <Tooltip label="Go to timecode or frame" shortcut={[modKey, "G"]}>
                  <button
                    onClick={() => {
                      pause()
                      setGotoOpen(true)
                    }}
                    className="timecode -mx-2 rounded-xl px-2 text-lg font-medium tracking-tight text-fg-1 transition-colors hover:bg-white/[0.06] @3xl:text-2xl"
                  >
                    {tc(frame)}
                  </button>
                </Tooltip>
              )}
              <span className="timecode hidden text-sm whitespace-nowrap text-fg-3 @xl:inline">
                {frame} / {last}
              </span>
            </div>

            <div className="flex items-center gap-1">
              <TransportButton label="First frame" keys={["Home"]} onClick={() => seek(0)}>
                <SkipBack weight="fill" />
              </TransportButton>
              <TransportButton label="Previous frame" keys={["←"]} onClick={() => step(-1)}>
                <CaretLineLeft weight="bold" />
              </TransportButton>
              <Tooltip label={playing ? "Pause" : "Play"} shortcut={["Space"]}>
                <button
                  onClick={toggle}
                  aria-label={playing ? "Pause" : "Play"}
                  className="mx-2 flex size-12 items-center justify-center rounded-full bg-cta text-on-cta shadow-[0_10px_30px_-8px_rgb(255_255_255/0.35)] transition-transform duration-[var(--dur-fast)] hover:scale-105 active:scale-95 [&_svg]:size-5"
                >
                  {playing ? <Pause weight="fill" /> : <Play weight="fill" className="translate-x-px" />}
                </button>
              </Tooltip>
              <TransportButton label="Next frame" keys={["→"]} onClick={() => step(1)}>
                <CaretLineRight weight="bold" />
              </TransportButton>
              <TransportButton label="Last frame" keys={["End"]} onClick={() => seek(last)}>
                <SkipForward weight="fill" />
              </TransportButton>
            </div>

            <div className="flex min-w-0 items-center justify-end gap-2">
              <MarkChip
                label="In"
                value={inFrame}
                tc={tc}
                onMark={() => markIn()}
                onGo={() => inFrame != null && seek(inFrame)}
                keyHint="I"
              />
              <MarkChip
                label="Out"
                value={outFrame}
                tc={tc}
                onMark={() => markOut()}
                onGo={() => outFrame != null && seek(outFrame)}
                keyHint="O"
              />
              {!inspector && (
                <Tooltip label="Export clip" shortcut={[modKey, "E"]}>
                  <Button
                    size="icon"
                    variant="primary"
                    aria-label="Export clip"
                    disabled={!canExport}
                    onClick={openExport}
                  >
                    <Scissors weight="bold" />
                  </Button>
                </Tooltip>
              )}
              <div className="ml-1 hidden items-center gap-1.5 @3xl:flex">
                <Tooltip label={muted ? "Unmute" : "Mute"} shortcut={["M"]}>
                  <button
                    onClick={toggleMute}
                    aria-label={muted ? "Unmute" : "Mute"}
                    className="flex size-9 items-center justify-center rounded-full text-fg-2 transition-colors hover:bg-white/[0.08] hover:text-fg-1 [&_svg]:size-[18px]"
                  >
                    {muted || volume === 0 ? <SpeakerSlash /> : volume < 0.5 ? <SpeakerLow /> : <SpeakerHigh />}
                  </button>
                </Tooltip>
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.05}
                  value={muted ? 0 : volume}
                  aria-label="Volume"
                  onChange={(e) => setVolume(Number(e.currentTarget.value))}
                  className="h-1 w-20 accent-[var(--fg-1)]"
                />
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-1.5 rounded-2xl border border-white/[0.07] bg-white/[0.03] px-4 pt-2 pb-3.5 [--lane-label-w:132px]">
            <TrackRow label="Playhead">
              <Scrubber
                clock={clock}
                frame={frame}
                inFrame={inFrame}
                outFrame={outFrame}
                onSeek={seek}
                onScrubStart={pause}
                formatFrame={tc}
              />
            </TrackRow>
            {shots.length > 0 && (
              <TrackRow label="Shots">
                <ShotStrip
                  segments={shots}
                  clock={clock}
                  frame={frame}
                  onSeek={(f) => {
                    pause()
                    seek(f)
                  }}
                  onMarkShot={markShot}
                  formatFrame={tc}
                />
              </TrackRow>
            )}
            {lanes.length > 0 && (
              <CharacterLanes
                lanes={lanes}
                clock={clock}
                focus={focus}
                onSeek={(f) => {
                  pause()
                  seek(f)
                }}
                formatFrame={tc}
              />
            )}
          </div>
        </div>
      </div>

      {inspector && (
        <aside className="flex w-[340px] shrink-0 flex-col gap-3 overflow-y-auto border-l border-white/[0.06] bg-canvas p-3">
          <Card
            title="Clip"
            action={
              hasMarks && (
                <Tooltip label="Clear in and out" shortcut={["Alt", "X"]}>
                  <button
                    onClick={clear}
                    aria-label="Clear in and out"
                    className="flex size-7 items-center justify-center rounded-full text-fg-3 hover:bg-white/[0.08] hover:text-fg-1"
                  >
                    <X className="size-3.5" weight="bold" />
                  </button>
                </Tooltip>
              )
            }
          >
            <div className="grid grid-cols-2 gap-2">
              <MarkTile
                label="In"
                keyHint="I"
                value={inFrame}
                tc={tc}
                onMark={() => markIn()}
                onGo={() => inFrame != null && seek(inFrame)}
              />
              <MarkTile
                label="Out"
                keyHint="O"
                value={outFrame}
                tc={tc}
                onMark={() => markOut()}
                onGo={() => outFrame != null && seek(outFrame)}
              />
            </div>
            <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
              <dt className="text-fg-3">Duration</dt>
              <dd className="timecode text-right text-fg-1">{hasMarks ? tc(length) : "–"}</dd>
              <dt className="text-fg-3">Frames</dt>
              <dd className="timecode text-right text-fg-1">{hasMarks ? length : "–"}</dd>
            </dl>
            <Button variant="primary" size="lg" className="mt-4 w-full" disabled={!canExport} onClick={openExport}>
              <Scissors weight="bold" /> Export clip
              <Kbd
                keys={[modKey, "E"]}
                className="ml-auto [&_kbd]:border-black/10 [&_kbd]:bg-black/10 [&_kbd]:text-black/60"
              />
            </Button>
            {!hasMarks && (
              <p className="mt-2.5 text-center text-xs text-fg-3">Mark an in or out point, or a whole shot (X).</p>
            )}
          </Card>

          <ShotsPanel
            pid={pid}
            media={m}
            segments={shots}
            frame={frame}
            tc={tc}
            onMark={() => markShot()}
            onGo={goShot}
          />

          <CharactersPanel
            pid={pid}
            media={m}
            lanes={lanes}
            shot={shotAt(shots, frame)}
            frame={frame}
            focus={focus}
            onFocus={setFocus}
            onGo={(f) => {
              pause()
              seek(f)
            }}
          />

          <Card title="Source">
            <p className="text-md font-medium text-fg-1">{releaseTitle(m.fileName)}</p>
            <p data-selectable className="mt-0.5 text-xs [overflow-wrap:anywhere] text-fg-3">
              {m.fileName}
            </p>
            {m.info && (
              <p className="timecode mt-2 text-xs text-fg-2">
                {m.info.video.width}×{m.info.video.height} · {formatFps(fps)} fps · {codecLabel(m.info.video.codec)}
              </p>
            )}
            {formatFlags(m).length > 0 && (
              <div className="mt-2.5 flex flex-wrap gap-1.5">
                {formatFlags(m).map((f) => (
                  <Badge key={f} tone="outline" className="h-5 px-2 text-2xs">
                    {f}
                  </Badge>
                ))}
              </div>
            )}
            {audio.length > 1 && (
              <div className="mt-3">
                <Select
                  size="sm"
                  aria-label="Audio track"
                  className="w-full"
                  value={String(currentAudio(m))}
                  onChange={(v) =>
                    void commands
                      .mediaSetAudio(pid, m.id, Number(v))
                      .catch((e) => toast.error("Couldn't change the audio track", { description: errorMessage(e) }))
                  }
                  options={audio.map(audioOption)}
                />
              </div>
            )}
            <Button size="sm" variant="ghost" className="mt-3 -ml-2" onClick={() => void commands.revealPath(m.path)}>
              <FolderOpen /> Show in folder
            </Button>
          </Card>

          <Card title="Keys">
            <ul className="space-y-2 text-sm text-fg-2">
              {PLAYER_SHORTCUTS.map((s) => (
                <li key={s.id} className="flex items-center justify-between gap-3">
                  <span>{s.label}</span>
                  <Kbd keys={s.keys} className="shrink-0" />
                </li>
              ))}
            </ul>
          </Card>
        </aside>
      )}

      <ExportDialog projectId={pid} clips={[clip]} notice={oneSided} open={exportOpen} onOpenChange={setExportOpen} />
    </div>
  )
}

/**
 * The picture's glow in the room: a tiny copy of the current frame, blurred
 * far beyond the screen's edges.
 */
function Ambilight({ video, frame }: { video: React.RefObject<HTMLVideoElement | null>; frame: number }) {
  const canvas = useRef<HTMLCanvasElement>(null)
  const last = useRef(0)
  // Playback moves `frame` every video frame; the glow follows ~10 times a
  // second (a trailing draw catches the frame it stops on).
  useEffect(() => {
    const wait = AMBILIGHT_MS - (performance.now() - last.current)
    const draw = () => {
      last.current = performance.now()
      drawFrame(video.current, canvas.current)
    }
    if (wait <= 0) {
      draw()
      return
    }
    const t = window.setTimeout(draw, wait)
    return () => window.clearTimeout(t)
  }, [video, frame])
  useEffect(() => {
    const v = video.current
    if (!v) return
    const draw = () => drawFrame(v, canvas.current)
    v.addEventListener("seeked", draw)
    v.addEventListener("loadeddata", draw)
    return () => {
      v.removeEventListener("seeked", draw)
      v.removeEventListener("loadeddata", draw)
    }
  }, [video])
  return (
    <canvas
      ref={canvas}
      width={48}
      height={27}
      aria-hidden
      className="pointer-events-none absolute inset-0 size-full scale-110 opacity-60 blur-[100px] brightness-125 saturate-[1.8]"
    />
  )
}

/** How often the ambilight follows playback. */
const AMBILIGHT_MS = 100

function drawFrame(v: HTMLVideoElement | null, c: HTMLCanvasElement | null) {
  const ctx = c?.getContext("2d")
  if (!v || !ctx || v.readyState < 2) return
  try {
    ctx.drawImage(v, 0, 0, ctx.canvas.width, ctx.canvas.height)
  } catch {
    // A frame that can't be read (no CORS) just leaves the room dark.
  }
}

/** A timeline row: its name in the label column, the track beside it. */
function TrackRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[var(--lane-label-w)_minmax(0,1fr)] items-center gap-3">
      <span className="text-xs font-medium text-fg-3">{label}</span>
      {children}
    </div>
  )
}

/** A titled card of the details panel. */
function Card({ title, action, children }: { title: string; action?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="rounded-2xl border border-white/[0.06] bg-white/[0.03] p-4">
      <div className="mb-3 flex min-h-7 items-center justify-between gap-2">
        <h2 className="text-sm font-semibold text-fg-1">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  )
}

/** The shot under the playhead, or where shot detection stands. */
function ShotsPanel({
  pid,
  media: m,
  segments,
  frame,
  tc,
  onMark,
  onGo,
}: {
  pid: string
  media: MediaItem
  segments: Segment[]
  frame: number
  tc: (f: number) => string
  onMark: () => void
  onGo: (f: number | null) => void
}) {
  const job = useMediaJob(m.id, "analyze")
  const analyzing = !!job
  const numbers = useMemo(() => shotNumbers(segments), [segments])
  const total = numbers.reduce((n, x) => (x >= 0 ? n + 1 : n), 0)
  const i = segmentIndexAt(segments, frame)
  const seg = i >= 0 ? segments[i] : null
  const analyze = () =>
    void commands
      .mediaAnalyze(pid, m.id, null)
      .catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))

  return (
    <Card
      title="Shot"
      action={m.shots === "ready" && <span className="timecode text-xs text-fg-3">{total} shots</span>}
    >
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
              Mark shot <Kbd keys={["X"]} className="ml-auto" />
            </Button>
            <Tooltip label="Previous shot" shortcut={["↑"]}>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="Previous shot"
                onClick={() => onGo(prevShotStart(segments, frame))}
              >
                <CaretLeft />
              </Button>
            </Tooltip>
            <Tooltip label="Next shot" shortcut={["↓"]}>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="Next shot"
                onClick={() => onGo(nextShotStart(segments, frame))}
              >
                <CaretRight />
              </Button>
            </Tooltip>
          </div>
        </>
      ) : analyzing ? (
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

/** Who is in the shot under the playhead, and everyone in this episode. */
function CharactersPanel({
  pid,
  media: m,
  lanes,
  shot,
  frame,
  focus,
  onFocus,
  onGo,
}: {
  pid: string
  media: MediaItem
  lanes: Lane[]
  shot: Segment | null
  frame: number
  focus: string | null
  onFocus: (id: string | null) => void
  onGo: (f: number) => void
}) {
  const job = useMediaJob(m.id, "analyze")
  const [all, setAll] = useState(false)
  const here = shot ? charactersIn(lanes, shot.start, shot.end) : charactersIn(lanes, frame, frame + 1)
  const hereIds = new Set(here.map((l) => l.character.id))
  // Who is in the shot, then the episode's main characters; the rest on request.
  const others = lanes.filter((l) => !hereIds.has(l.character.id))
  const listed = [...here, ...(all ? others : others.slice(0, Math.max(0, 8 - here.length)))]
  const hiddenCount = here.length + others.length - listed.length
  /** The next time `lane` is on screen after the playhead (wrapping around). */
  const next = (lane: Lane) => (lane.spans.find((s) => s.start > frame) ?? lane.spans[0])?.start
  return (
    <Card
      title="Characters"
      action={
        <Link to="/p/$pid/cast" params={{ pid }} className="text-xs font-medium text-fg-3 hover:text-fg-1">
          Cast
        </Link>
      }
    >
      {m.faces !== "ready" ? (
        job ? (
          <div>
            <div className="flex justify-between text-xs text-fg-3">
              <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "Analysing")}</span>
              <span className="timecode">{formatPercent(job.progress)}</span>
            </div>
            <Progress className="mt-1.5" value={job.state === "running" ? job.progress : null} />
          </div>
        ) : (
          <p className="text-sm text-fg-3">
            {m.faces === "failed"
              ? (m.facesError ?? "Finding characters failed.")
              : "Characters haven't been found in this episode yet."}
          </p>
        )
      ) : lanes.length === 0 ? (
        <p className="text-sm text-fg-3">Nobody the cast knows appears here.</p>
      ) : (
        <ul className="-mx-1.5 flex flex-col gap-0.5">
          {listed.map((l) => {
            const c = l.character
            const on = hereIds.has(c.id)
            return (
              <li key={c.id}>
                <button
                  onClick={() => onFocus(focus === c.id ? null : c.id)}
                  onDoubleClick={() => {
                    const f = next(l)
                    if (f != null) onGo(f)
                  }}
                  title="Click to highlight · double-click for the next appearance"
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-xl px-1.5 py-1.5 text-left transition-colors hover:bg-white/[0.06]",
                    focus === c.id && "bg-accent-subtle",
                  )}
                >
                  <span
                    className={cn(
                      "size-8 shrink-0 overflow-hidden rounded-full bg-inset",
                      on ? "ring-2 ring-accent/70" : "opacity-50",
                    )}
                  >
                    {c.coverSmall && <img src={assetUrl(c.coverSmall)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className={cn("min-w-0 flex-1 truncate text-sm font-medium", on ? "text-fg-1" : "text-fg-3")}>
                    {c.label}
                  </span>
                  {on && <span className="text-xs text-accent">in shot</span>}
                </button>
              </li>
            )
          })}
          {(hiddenCount > 0 || all) && (
            <li>
              <button
                onClick={() => setAll((v) => !v)}
                className="mt-0.5 w-full rounded-xl px-1.5 py-1.5 text-left text-xs font-medium text-fg-3 hover:bg-white/[0.06] hover:text-fg-1"
              >
                {all ? "Show fewer" : `${hiddenCount} more in this episode`}
              </button>
            </li>
          )}
        </ul>
      )}
      {m.faces === "ready" && lanes.length > 0 && (
        <Button size="sm" variant="ghost" className="mt-2 -ml-2" asChild>
          <Link to="/p/$pid/cast" params={{ pid }}>
            <Users /> Edit the cast
          </Link>
        </Button>
      )}
    </Card>
  )
}

const SEGMENT_LABEL: Record<Segment["kind"], string> = {
  shot: "Shot",
  dissolve: "Dissolve",
  fade_out: "Fade out",
  fade_in: "Fade in",
  black: "Black",
  flash: "Flash",
}

function TransportButton({
  label,
  keys,
  onClick,
  children,
}: {
  label: string
  keys: string[]
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <Tooltip label={label} shortcut={keys}>
      <button
        onClick={onClick}
        aria-label={label}
        className="flex size-10 items-center justify-center rounded-full text-fg-2 transition-colors hover:bg-white/[0.08] hover:text-fg-1 [&_svg]:size-[18px]"
      >
        {children}
      </button>
    </Tooltip>
  )
}

/** In or out point in the details panel: set it, or go to it. */
function MarkTile({
  label,
  keyHint,
  value,
  tc,
  onMark,
  onGo,
}: {
  label: string
  keyHint: string
  value: number | null
  tc: (f: number) => string
  onMark: () => void
  onGo: () => void
}) {
  return (
    <div
      className={cn(
        "flex flex-col rounded-xl border px-3 py-2.5",
        value != null ? "border-accent-line bg-accent-subtle" : "border-white/[0.08] bg-black/20",
      )}
    >
      <div className="flex items-center justify-between">
        <span className={cn("text-xs font-semibold", value != null ? "text-accent" : "text-fg-3")}>{label}</span>
        <Kbd keys={[keyHint]} />
      </div>
      {value != null ? (
        <button
          onClick={onGo}
          className="timecode mt-1.5 text-left text-md text-fg-1 hover:text-accent"
          title="Go to this frame"
        >
          {tc(value)}
        </button>
      ) : (
        <button onClick={onMark} className="mt-1.5 text-left text-md text-fg-3 hover:text-fg-1">
          Mark {label.toLowerCase()}
        </button>
      )}
    </div>
  )
}

/** In or out point in the transport row: gold once set. */
function MarkChip({
  label,
  keyHint,
  value,
  tc,
  onMark,
  onGo,
}: {
  label: string
  keyHint: string
  value: number | null
  tc: (f: number) => string
  onMark: () => void
  onGo: () => void
}) {
  return (
    <Tooltip
      label={value != null ? `Go to ${label.toLowerCase()} point` : `Mark ${label.toLowerCase()}`}
      shortcut={value != null ? ["Shift", keyHint] : [keyHint]}
    >
      <button
        onClick={value != null ? onGo : onMark}
        className={cn(
          "flex h-9 items-center gap-2 rounded-full border px-3 text-sm whitespace-nowrap transition-colors",
          value != null
            ? "border-accent-line bg-accent-subtle text-fg-1 hover:bg-accent/25"
            : "border-white/10 text-fg-3 hover:border-white/20 hover:text-fg-1",
        )}
      >
        <span className={cn("text-xs font-semibold", value != null && "text-accent")}>{label}</span>
        {value != null && <span className="timecode hidden @5xl:inline">{tc(value)}</span>}
      </button>
    </Tooltip>
  )
}
