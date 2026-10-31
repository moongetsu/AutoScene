import { Link } from "@tanstack/react-router"
import { FileX, FilmStrip, Play, Warning, X } from "@/components/icons"
import type { MediaItem } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import { ContextArea } from "@/components/ui/context-area"
import { contextKit } from "@/components/ui/menu-actions"
import { ProgressRing } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { formatDuration } from "@/lib/format"
import { cancelJob, cancelLabel } from "@/lib/jobs"
import { releaseTitle } from "@/lib/media"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"
import { EpisodeItems } from "./episode-actions"

/** An episode as its still: code and title under it, its state on it. Its right-click is the episode's own. */
export function EpisodeCard({
  pid,
  media: m,
  onOpen,
  onRemove,
  className,
}: {
  pid: string
  media: MediaItem
  onOpen: () => void
  onRemove: () => void
  className?: string
}) {
  const job = useMediaJob(m.id)
  const working = job && job.state === "running"
  const broken = m.status === "missing" || m.status === "failed"
  return (
    <ContextArea menu={<EpisodeItems kit={contextKit} projectId={pid} media={m} onOpen={onOpen} onRemove={onRemove} />}>
      <div className={cn("group relative rounded-2xl", className)}>
        <Tooltip label={m.fileName} side="top" delay={600}>
          <Link to="/p/$pid/timeline/$mid" params={{ pid, mid: m.id }} className="block rounded-2xl">
            <Artwork
              path={m.posterSmall ?? m.poster}
              className="aspect-video rounded-xl picture-edge transition-[box-shadow,transform] duration-[var(--dur-med)] ease-out group-hover:-translate-y-1 group-hover:shadow-lift"
              imgClassName={cn("group-hover:scale-[1.04] duration-[600ms]", broken && "opacity-40 grayscale")}
              fallback={
                <div className="flex size-full items-center justify-center artwork-fallback text-fg-disabled [&_svg]:size-8">
                  {m.status === "missing" ? <FileX /> : <FilmStrip />}
                </div>
              }
            >
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/60 via-transparent to-transparent opacity-80" />
              {m.label && (
                <Badge tone="media" className="timecode absolute top-2.5 left-2.5">
                  {m.label}
                </Badge>
              )}
              {m.durationSec ? (
                <span className="timecode absolute right-3 bottom-2.5 text-xs text-white/85">{formatDuration(m.durationSec)}</span>
              ) : null}
              {m.status === "missing" && (
                <Badge tone="danger" className="absolute right-2.5 top-2.5 bg-black/60 backdrop-blur-md">
                  <FileX /> Missing
                </Badge>
              )}
              {m.status === "failed" && (
                <Badge tone="danger" className="absolute right-2.5 top-2.5 bg-black/60 backdrop-blur-md">
                  <Warning /> Failed
                </Badge>
              )}
              {!broken && !working && m.status === "ready" && (
                <span className="absolute inset-0 m-auto flex size-12 scale-90 items-center justify-center rounded-full border border-white/20 bg-black/40 text-white opacity-0 backdrop-blur-md transition-[opacity,transform] duration-[var(--dur-med)] group-hover:scale-100 group-hover:opacity-100 [&_svg]:size-5">
                  <Play weight="fill" />
                </span>
              )}
            </Artwork>
            <div className="mt-3 px-0.5">
              <div className="truncate text-md font-medium text-fg-1">{releaseTitle(m.fileName)}</div>
              <div className="mt-0.5 truncate text-sm text-fg-3">
                {working ? (job.stage ?? "Working…") : m.shotCount != null ? `${m.shotCount.toLocaleString("en-GB")} shots` : "Not analysed yet"}
                {m.faceCount ? ` · ${m.faceCount.toLocaleString("en-GB")} faces` : ""}
              </div>
            </div>
          </Link>
        </Tooltip>
        {working && (
          // Over the still and moving with it, outside the link so its
          // cancel is a button of its own; the rest lets clicks through to
          // the link.
          <div className="pointer-events-none absolute inset-x-0 top-0 flex aspect-video items-center justify-center p-3 transition-transform duration-[var(--dur-med)] ease-out group-hover:-translate-y-1">
            <div className="relative">
              <div className="flex items-center gap-2 rounded-full border border-white/15 bg-black/60 px-3.5 py-1.5 shadow-2xl backdrop-blur-md">
                <ProgressRing value={job.progress} aria-label={job.stage ?? "Working"} />
                <span className="timecode text-xs font-semibold tracking-wider text-fg-1">
                  {job.progress != null ? `${Math.round(job.progress * 100)}%` : "Working…"}
                </span>
              </div>
              <Tooltip label={cancelLabel(job.kind)}>
                <button
                  aria-label={cancelLabel(job.kind)}
                  onClick={() => cancelJob(job)}
                  className="pointer-events-auto absolute top-1/2 left-full ml-2 flex size-8 -translate-y-1/2 items-center justify-center rounded-full border border-white/15 bg-black/60 text-fg-2 opacity-0 shadow-2xl backdrop-blur-md transition-opacity group-hover:opacity-100 hover:bg-black/75 hover:text-fg-1 focus-visible:opacity-100"
                >
                  <X className="size-3.5" />
                </button>
              </Tooltip>
            </div>
          </div>
        )}
      </div>
    </ContextArea>
  )
}
