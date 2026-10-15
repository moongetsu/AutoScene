import type * as React from "react"
import { Link } from "@tanstack/react-router"
import { ArrowCounterClockwise, ArrowLeft, CircleNotch, MagnifyingGlass } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { locateFile } from "@/features/project/relink"
import { formatPercent } from "@/lib/format"
import { commands, errorMessage, isAppError } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { useMediaJob } from "@/state/activity"

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
