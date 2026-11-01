import type * as React from "react"
import { useState } from "react"
import { Link } from "@tanstack/react-router"
import { ArrowCounterClockwise, ArrowLeft, CircleNotch, MagnifyingGlass, X } from "@/components/icons"
import { toast } from "sonner"
import type { FrameCount, JobInfo, MediaItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { locateFile } from "@/features/project/relink"
import { formatCount, formatEta, formatPace, formatPercent, formatProgress } from "@/lib/format"
import { commands, errorMessage, isAppError } from "@/lib/ipc"
import { cancelJob, cancelLabel } from "@/lib/jobs"
import { assetUrl } from "@/lib/media"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"
import { useIndexingStill } from "./indexing-still"

/** Poster with what's happening instead of a player. */
export function NotReady({
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
  const needsPreview = m?.status === "ready" && isAppError(error) && error.code === "not_ready"
  // The job playback waits for — its indexing or its preview — not an
  // analysis running meanwhile: its progress shows, and Cancel stops it.
  const ingest = useMediaJob(m?.id, "ingest")
  const preview = useMediaJob(m?.id, "proxy")
  const job = needsPreview ? preview : ingest
  const indexing = (m?.status === "queued" || m?.status === "indexing") && job?.kind === "ingest"
  // Where the indexing is, as a picture, asked for while it reads the frames
  // (there is no place to show before): it lights the room, as a poster would.
  const still = useIndexingStill(pid, m?.id, indexing && job.state === "running" && job.frames != null)
  const backdrop = still ?? (m?.poster ? assetUrl(m.posterSmall ?? m.poster) : null)
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
    body = `${m.path} isn't there any more. Moved it? Show AutoScene where it is: its analysis and your decisions stay.`
  } else if (needsPreview) {
    title = job ? "Preparing preview" : m?.proxy === "failed" ? "Preview failed" : "Preview not prepared"
    body = job
      ? "This format needs a lightweight preview copy for frame-accurate playback. Exports still cut from the original."
      : error.message
  } else if (error) {
    title = "Can't play this file"
    body = errorMessage(error)
  }
  const retryable = m?.status === "failed"
  return (
    <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden">
      {backdrop && (
        <div aria-hidden className="absolute inset-0 scale-110 opacity-30 blur-2xl">
          <Crossfade src={backdrop} />
        </div>
      )}
      <div className="glass-strong relative flex w-[min(460px,90%)] flex-col items-center rounded-3xl px-8 py-8 text-center">
        {indexing ? (
          <div className="picture-edge artwork-fallback relative grid aspect-video w-64 place-items-center overflow-hidden rounded-xl">
            {!still && <CircleNotch className="size-6 animate-spin text-accent" />}
            <Crossfade src={still} />
          </div>
        ) : job ? (
          <CircleNotch className="size-6 animate-spin text-accent" />
        ) : null}
        <h2 className={cn("display text-3xl text-fg-1", indexing ? "mt-5" : "mt-3")}>{title}</h2>
        {body && <p className="mt-2 text-md break-words text-fg-2">{body}</p>}
        {job && (
          <div className="mt-6 w-full">
            <Progress value={job.state === "running" ? job.progress : null} />
            {job.state === "running" && job.frames ? (
              <FramesRead job={job} frames={job.frames} />
            ) : (
              <div className="mt-2 flex justify-between gap-2 text-xs text-fg-3">
                <span className="truncate">{job.state === "queued" ? "Waiting…" : (job.stage ?? "")}</span>
                <span className="timecode shrink-0">{formatProgress(job.progress, job.etaMs)}</span>
              </div>
            )}
            {/* Indexing keeps a line for its pace, so the card doesn't jump as its frames start and end. */}
            {indexing && !(job.state === "running" && job.frames) && <div aria-hidden className="mt-1 h-4" />}
            <Button className="mt-5" size="sm" onClick={() => cancelJob(job)}>
              <X /> {cancelLabel(job.kind)}
            </Button>
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
        {m?.status === "missing" && (
          <Button className="mt-6" variant="primary" onClick={() => void locateFile(pid, m)}>
            <MagnifyingGlass /> Locate file…
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

/**
 * The frames an indexing has read, of about how many, and how fast. The
 * pace and the time left show once they are measured, never guessed: a
 * first burst says little about either.
 */
function FramesRead({ job, frames }: { job: JobInfo; frames: FrameCount }) {
  const { done, total, rate, perSec } = frames
  return (
    <div className="mt-2 flex flex-col gap-1 text-xs text-fg-3">
      <div className="flex justify-between gap-2">
        <span className="truncate">
          <span className="timecode text-fg-2">{formatCount(done)}</span>
          {/* The file's own count can be off: past it, only what was read is told. */}
          {total != null && total >= done ? (
            <>
              {" of about "}
              <span className="timecode">{formatCount(total)}</span>
              {" frames"}
            </>
          ) : (
            " frames"
          )}
        </span>
        <span className="timecode shrink-0">{formatPercent(job.progress)}</span>
      </div>
      <div className="flex h-4 justify-between gap-2">
        {perSec == null ? (
          <span className="truncate">Measuring speed…</span>
        ) : (
          <span className="truncate">
            <span className="timecode">{formatPace(perSec)}</span>
            {" fps"}
            {rate && perSec > 0 ? (
              <>
                {" · "}
                <span className="timecode">{formatPace(perSec / rate)}×</span>
                {" real time"}
              </>
            ) : null}
          </span>
        )}
        <span className="timecode shrink-0">{perSec == null ? "" : formatEta(job.etaMs)}</span>
      </div>
    </div>
  )
}

/** `src` fading in over the picture before it once loaded (the first one over nothing). */
function Crossfade({ src }: { src: string | null }) {
  const [loaded, setLoaded] = useState<{ now: string | null; before: string | null }>({ now: null, before: null })
  // The one on screen stays underneath until `src` has faded in over it.
  const under = loaded.now !== src ? loaded.now : loaded.before
  return (
    <>
      {under && (
        <img key={under} src={under} alt="" draggable={false} className="absolute inset-0 size-full object-cover" />
      )}
      {src && (
        <img
          key={src}
          src={src}
          alt=""
          draggable={false}
          decoding="async"
          onLoad={() => setLoaded((l) => (l.now === src ? l : { now: src, before: l.now }))}
          className={cn(
            "absolute inset-0 size-full object-cover transition-opacity duration-[var(--dur-slow)] ease-out",
            loaded.now === src ? "opacity-100" : "opacity-0",
          )}
        />
      )}
    </>
  )
}
