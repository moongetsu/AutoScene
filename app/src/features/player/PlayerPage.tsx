import type * as React from "react"
import { useEffect, useMemo, useRef, useState } from "react"
import { Link, useParams } from "@tanstack/react-router"
import {
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Loader2,
  Pause,
  Play,
  RotateCcw,
  Scissors,
  SkipBack,
  SkipForward,
  StepBack,
  StepForward,
  Volume1,
  Volume2,
  VolumeX,
  X,
} from "lucide-react"
import { toast } from "sonner"
import type { FrameMap, MediaItem, Segment } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Progress } from "@/components/ui/progress"
import { Select } from "@/components/ui/select"
import { Tooltip } from "@/components/ui/tooltip"
import { ExportDialog, type ExportClip } from "@/features/export/ExportDialog"
import { COPY_TIMECODES_REQUEST, EXPORT_REQUEST, REVEAL_REQUEST } from "@/lib/actions"
import { FrameClock } from "@/lib/frame-clock"
import { formatPercent } from "@/lib/format"
import { commands, errorMessage, isAppError } from "@/lib/ipc"
import { assetUrl, audioOption, codecLabel, currentAudio, fileStem, formatFlags, mediaTitle, releaseTitle } from "@/lib/media"
import { modKey } from "@/lib/platform"
import { useFrameMap, useMediaList, useProject, useSegments } from "@/lib/queries"
import { nextShotStart, prevShotStart, segmentIndexAt, shotAt, shotNumbers } from "@/lib/segments"
import { formatFps, parseTimecode, timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"
import { NO_MARKS, useMarks } from "@/state/marks"
import { usePlayback } from "@/state/playback"
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
  const { data: media } = useMediaList(pid)
  const m = media?.find((x) => x.id === mid)
  const ready = m?.status === "ready"
  const map = useFrameMap(pid, mid, ready)
  const index = media?.findIndex((x) => x.id === mid) ?? -1
  const prev = index > 0 ? media![index - 1] : undefined
  const next = index >= 0 && index < (media?.length ?? 0) - 1 ? media![index + 1] : undefined

  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-hairline px-3">
          <EpisodeNav pid={pid} target={prev} dir="prev" />
          <EpisodeNav pid={pid} target={next} dir="next" />
          <div className="ml-1 flex min-w-0 items-baseline gap-2">
            {m?.label && <span className="timecode text-sm text-fg-1">{m.label}</span>}
            <span className="truncate text-sm text-fg-2" title={m?.fileName}>
              {m ? releaseTitle(m.fileName) : ""}
            </span>
          </div>
          {map.data?.isProxy && (
            <Tooltip label="Playing a preview copy; exports always cut from the original file.">
              <span className="ml-auto">
                <Badge tone="outline">Preview</Badge>
              </span>
            </Tooltip>
          )}
        </header>
        {m && ready && map.data ? (
          <ReadyPlayer key={`${mid}:${map.data.src}`} pid={pid} media={m} map={map.data} />
        ) : (
          <NotReady pid={pid} media={m} error={map.error} />
        )}
      </div>
    </div>
  )
}

function EpisodeNav({ pid, target, dir }: { pid: string; target?: MediaItem; dir: "prev" | "next" }) {
  const label = dir === "prev" ? "Previous episode" : "Next episode"
  const icon = dir === "prev" ? <ChevronLeft /> : <ChevronRight />
  if (!target) {
    return (
      <Button size="icon-sm" variant="ghost" disabled aria-label={label}>
        {icon}
      </Button>
    )
  }
  return (
    <Tooltip label={`${label}: ${mediaTitle(target)}`}>
      <Button size="icon-sm" variant="ghost" asChild>
        <Link to="/p/$pid/timeline/$mid" params={{ pid, mid: target.id }} aria-label={label}>
          {icon}
        </Link>
      </Button>
    </Tooltip>
  )
}

/** Poster with what's happening instead of a player. */
function NotReady({ pid, media: m, error }: { pid: string; media?: MediaItem; error: unknown }) {
  const job = useMediaJob(m?.id)
  const needsPreview = m?.status === "ready" && isAppError(error) && error.code === "not_ready"
  let title = "Loading…"
  let body: React.ReactNode = null
  if (m?.status === "queued" || m?.status === "indexing") {
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
    <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-inset">
      {m?.poster && <img src={assetUrl(m.poster)} alt="" className="absolute inset-0 size-full object-contain opacity-25 blur-[2px]" />}
      <div className="relative flex w-[min(420px,90%)] flex-col items-center rounded-xl border border-hairline bg-raised px-6 py-6 text-center shadow-2">
        {job ? <Loader2 className="size-5 animate-spin text-fg-2" /> : null}
        <h2 className="mt-2 text-md font-semibold text-fg-1">{title}</h2>
        {body && <p className="mt-1 text-sm break-words text-fg-2">{body}</p>}
        {job && (
          <div className="mt-4 w-full">
            <Progress value={job.state === "running" ? job.progress : null} />
            <div className="mt-1.5 flex justify-between text-2xs text-fg-3">
              <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "")}</span>
              <span className="timecode">{formatPercent(job.progress)}</span>
            </div>
          </div>
        )}
        {!job && needsPreview && (
          <Button className="mt-4" onClick={() => void commands.mediaPreparePreview(pid, m!.id).catch((e) => toast.error(errorMessage(e)))}>
            <RotateCcw /> Prepare preview
          </Button>
        )}
        {!job && retryable && (
          <Button className="mt-4" onClick={() => void commands.mediaRetry(pid, m.id).catch((e) => toast.error(errorMessage(e)))}>
            <RotateCcw /> Try again
          </Button>
        )}
      </div>
    </div>
  )
}

function ReadyPlayer({ pid, media: m, map }: { pid: string; media: MediaItem; map: FrameMap }) {
  const clock = useMemo(() => new FrameClock(map), [map])
  const { videoRef, frame, playing, seek, step, toggle, pause } = usePlayer(clock)
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

  const clip: ExportClip = {
    mediaId: m.id,
    start,
    end,
    name: null,
    summary: `${m.label ? `${m.label} · ` : ""}${tc(start)} – ${tc(end - 1)} · ${length} frames`,
    suggestedName: suggestedName(project?.title ?? "", m, tc(start)),
  }
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
      else if (e.key === "ArrowUp") goShot(prevShotStart(shots, frame))
      else if (e.key === "ArrowDown") goShot(nextShotStart(shots, frame))
      else if (e.key === "Home") seek(0)
      else if (e.key === "End") seek(last)
      else if (k === "i" && e.shiftKey) {
        if (inFrame != null) seek(inFrame)
      } else if (k === "o" && e.shiftKey) {
        if (outFrame != null) seek(outFrame)
      }
      else if (k === "i") markIn()
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

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="relative flex min-h-0 flex-1 items-center justify-center bg-inset" onClick={toggle}>
          <video
            ref={videoRef}
            src={assetUrl(map.src)}
            preload="auto"
            // The asset protocol answers with CORS headers; keeps frames readable for canvas grabs.
            crossOrigin="anonymous"
            playsInline
            disablePictureInPicture
            className="size-full object-contain"
          />
        </div>

        {/* Secondary readouts fold away as the player column narrows. */}
        <div className="@container shrink-0 border-t border-hairline bg-panel px-4 pt-2 pb-3">
          <Scrubber
            clock={clock}
            frame={frame}
            inFrame={inFrame}
            outFrame={outFrame}
            onSeek={seek}
            onScrubStart={pause}
            formatFrame={tc}
          />
          {shots.length > 0 && (
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
          )}
          <div className="mt-1 grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4">
            <div className="flex min-w-0 items-baseline gap-3">
              {gotoOpen ? (
                <input
                  ref={gotoRef}
                  aria-label="Go to timecode or frame"
                  defaultValue={tc(frame)}
                  spellCheck={false}
                  className="timecode h-8 w-[150px] rounded-md border border-accent-line bg-inset px-2 text-xl text-fg-1 outline-none"
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
                    className="timecode -mx-1 rounded-sm px-1 text-xl text-fg-1 hover:bg-hover"
                  >
                    {tc(frame)}
                  </button>
                </Tooltip>
              )}
              <span className="timecode hidden text-xs whitespace-nowrap text-fg-3 @xl:inline">
                {frame} / {last}
              </span>
            </div>
            <div className="flex items-center gap-1">
              <TransportButton label="First frame" keys={["Home"]} onClick={() => seek(0)}>
                <SkipBack />
              </TransportButton>
              <TransportButton label="Previous frame" keys={["←"]} onClick={() => step(-1)}>
                <StepBack />
              </TransportButton>
              <Tooltip label={playing ? "Pause" : "Play"} shortcut={["Space"]}>
                <button
                  onClick={toggle}
                  aria-label={playing ? "Pause" : "Play"}
                  className="mx-1 flex size-10 items-center justify-center rounded-full bg-fg-1 text-canvas shadow-1 transition-colors hover:bg-fg-2 [&_svg]:size-4"
                >
                  {playing ? <Pause className="fill-current" /> : <Play className="translate-x-px fill-current" />}
                </button>
              </Tooltip>
              <TransportButton label="Next frame" keys={["→"]} onClick={() => step(1)}>
                <StepForward />
              </TransportButton>
              <TransportButton label="Last frame" keys={["End"]} onClick={() => seek(last)}>
                <SkipForward />
              </TransportButton>
            </div>
            <div className="flex min-w-0 items-center justify-end gap-2">
              <span className="timecode hidden text-xs whitespace-nowrap text-fg-3 @3xl:inline">
                {tc(last)} · {formatFps(fps)} fps
              </span>
              <div className="ml-2 flex items-center gap-1">
                <Tooltip label={muted ? "Unmute" : "Mute"} shortcut={["M"]}>
                  <button
                    onClick={toggleMute}
                    aria-label={muted ? "Unmute" : "Mute"}
                    className="flex size-8 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 [&_svg]:size-4"
                  >
                    {muted || volume === 0 ? <VolumeX /> : volume < 0.5 ? <Volume1 /> : <Volume2 />}
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
                  className="hidden h-1 w-20 cursor-pointer accent-[var(--fg-1)] @2xl:block"
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      <aside className="flex w-[300px] shrink-0 flex-col overflow-y-auto border-l border-hairline bg-panel">
        <section className="border-b border-hairline p-4">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="eyebrow">Clip</h2>
            {hasMarks && (
              <Tooltip label="Clear in and out" shortcut={["Alt", "X"]}>
                <button onClick={clear} aria-label="Clear in and out" className="rounded-sm p-1 text-fg-3 hover:bg-hover hover:text-fg-1">
                  <X className="size-3.5" />
                </button>
              </Tooltip>
            )}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <MarkButton label="In" keyHint="I" value={inFrame} tc={tc} onMark={() => markIn()} onGo={() => inFrame != null && seek(inFrame)} />
            <MarkButton label="Out" keyHint="O" value={outFrame} tc={tc} onMark={() => markOut()} onGo={() => outFrame != null && seek(outFrame)} />
          </div>
          <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
            <dt className="text-fg-3">Duration</dt>
            <dd className="timecode text-right text-fg-1">{hasMarks ? tc(length) : "–"}</dd>
            <dt className="text-fg-3">Frames</dt>
            <dd className="timecode text-right text-fg-1">{hasMarks ? length : "–"}</dd>
          </dl>
          <Button
            variant="primary"
            className="mt-4 w-full"
            disabled={!canExport}
            onClick={() => {
              pause()
              setExportOpen(true)
            }}
          >
            <Scissors /> Export clip
            <Kbd keys={[modKey, "E"]} className="ml-auto opacity-70 [&_kbd]:border-on-accent/20 [&_kbd]:bg-on-accent/10 [&_kbd]:text-on-accent" />
          </Button>
          {!hasMarks && <p className="mt-2 text-center text-2xs text-fg-3">Mark an in or out point to export.</p>}
        </section>

        <ShotsPanel pid={pid} media={m} segments={shots} frame={frame} tc={tc} onMark={() => markShot()} onGo={goShot} />

        <section className="border-b border-hairline p-4">
          <h2 className="eyebrow mb-3">Source</h2>
          <p className="text-sm text-fg-1">{releaseTitle(m.fileName)}</p>
          <p data-selectable className="mt-0.5 text-2xs [overflow-wrap:anywhere] text-fg-3">
            {m.fileName}
          </p>
          {m.info && (
            <p className="timecode mt-1 text-2xs text-fg-3">
              {m.info.video.width}×{m.info.video.height} · {formatFps(fps)} · {codecLabel(m.info.video.codec)}
            </p>
          )}
          {formatFlags(m).length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1">
              {formatFlags(m).map((f) => (
                <Badge key={f} tone="outline">
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
        </section>

        <section className="p-4">
          <h2 className="eyebrow mb-2.5">Keys</h2>
          <ul className="space-y-1.5 text-xs text-fg-2">
            {[
              ["Play / pause", ["Space"]],
              ["Step one frame", ["←", "→"]],
              ["Step one second", ["Shift", "←"]],
              ["Previous / next shot", ["↑", "↓"]],
              ["Mark shot", ["X"]],
              ["Mark in / out", ["I", "O"]],
              ["Go to in / out", ["Shift", "I"]],
              ["Go to timecode", [modKey, "G"]],
              ["Mute", ["M"]],
            ].map(([label, keys]) => (
              <li key={label as string} className="flex items-center justify-between">
                <span>{label}</span>
                <Kbd keys={keys as string[]} />
              </li>
            ))}
          </ul>
        </section>
      </aside>

      <ExportDialog projectId={pid} clips={[clip]} open={exportOpen} onOpenChange={setExportOpen} />
    </div>
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
  const job = useMediaJob(m.id)
  const analyzing = job?.kind === "analyze"
  const numbers = useMemo(() => shotNumbers(segments), [segments])
  const total = numbers.reduce((n, x) => (x >= 0 ? n + 1 : n), 0)
  const i = segmentIndexAt(segments, frame)
  const seg = i >= 0 ? segments[i] : null
  const analyze = () =>
    void commands.mediaAnalyze(pid, m.id, false).catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))

  return (
    <section className="border-b border-hairline p-4">
      <div className="mb-3 flex items-center justify-between">
        <h2 className="eyebrow">Shot</h2>
        {m.shots === "ready" && <span className="timecode text-2xs text-fg-3">{total} shots</span>}
      </div>
      {m.shots === "ready" && seg ? (
        <>
          <div className="flex items-baseline justify-between gap-2">
            <span className="text-sm text-fg-1">
              {seg.kind === "shot" ? `Shot ${numbers[i] + 1}` : SEGMENT_LABEL[seg.kind]}
              {seg.tiny && <span className="text-fg-3"> · tiny</span>}
            </span>
            <span className="timecode text-xs text-fg-2">{seg.end - seg.start} frames</span>
          </div>
          <p className="timecode mt-0.5 text-2xs text-fg-3">
            {tc(seg.start)} – {tc(seg.end - 1)}
          </p>
          <div className="mt-3 flex gap-2">
            <Button size="sm" variant="secondary" className="flex-1" onClick={onMark}>
              Mark shot <Kbd keys={["X"]} className="ml-auto" />
            </Button>
            <Tooltip label="Previous shot" shortcut={["↑"]}>
              <Button size="icon-sm" variant="ghost" aria-label="Previous shot" onClick={() => onGo(prevShotStart(segments, frame))}>
                <ChevronLeft />
              </Button>
            </Tooltip>
            <Tooltip label="Next shot" shortcut={["↓"]}>
              <Button size="icon-sm" variant="ghost" aria-label="Next shot" onClick={() => onGo(nextShotStart(segments, frame))}>
                <ChevronRight />
              </Button>
            </Tooltip>
          </div>
        </>
      ) : analyzing ? (
        <div>
          <div className="flex justify-between text-2xs text-fg-3">
            <span>{job.state === "queued" ? "Waiting…" : (job.stage ?? "Finding shots")}</span>
            <span className="timecode">{formatPercent(job.progress)}</span>
          </div>
          <Progress className="mt-1.5" value={job.state === "running" ? job.progress : null} />
        </div>
      ) : m.shots === "failed" ? (
        <div>
          <p className="text-xs text-danger">Shot detection failed.</p>
          {m.shotsError && <p className="mt-0.5 text-2xs break-words text-fg-3">{m.shotsError}</p>}
          <Button size="sm" variant="ghost" className="mt-2 -ml-2" onClick={analyze}>
            <RotateCcw /> Try again
          </Button>
        </div>
      ) : (
        <div>
          <p className="text-xs text-fg-3">Shots haven't been detected yet.</p>
          <Button size="sm" variant="ghost" className="mt-2 -ml-2" onClick={analyze}>
            <Scissors /> Find shots
          </Button>
        </div>
      )}
    </section>
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
        className="flex size-8 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 [&_svg]:size-4"
      >
        {children}
      </button>
    </Tooltip>
  )
}

function MarkButton({
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
        "flex flex-col rounded-md border bg-inset px-2.5 py-2",
        value != null ? "border-accent-line" : "border-hairline",
      )}
    >
      <div className="flex items-center justify-between">
        <span className="text-2xs font-medium text-fg-3">{label}</span>
        <Kbd keys={[keyHint]} />
      </div>
      {value != null ? (
        <button onClick={onGo} className="timecode mt-1 text-left text-sm text-fg-1 hover:text-accent" title="Go to this frame">
          {tc(value)}
        </button>
      ) : (
        <button onClick={onMark} className="mt-1 text-left text-sm text-fg-3 hover:text-fg-1">
          Mark {label.toLowerCase()}
        </button>
      )}
    </div>
  )
}
