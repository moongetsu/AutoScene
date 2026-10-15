import { Link } from "@tanstack/react-router"
import { FileX, FilmStrip, Play, Warning } from "@/components/icons"
import type { MediaItem } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import { Progress } from "@/components/ui/progress"
import { formatDuration } from "@/lib/format"
import { releaseTitle } from "@/lib/media"
import { cn } from "@/lib/utils"
import { useMediaJob } from "@/state/activity"

/** An episode as its still: code and title under it, its state on it. */
export function EpisodeCard({ pid, media: m, className }: { pid: string; media: MediaItem; className?: string }) {
  const job = useMediaJob(m.id)
  const working = job && job.state === "running"
  const broken = m.status === "missing" || m.status === "failed"
  return (
    <Link
      to="/p/$pid/timeline/$mid"
      params={{ pid, mid: m.id }}
      className={cn("group block rounded-2xl", className)}
      title={m.fileName}
    >
      <Artwork
        path={m.poster}
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
        {!broken && m.status === "ready" && (
          <span className="absolute inset-0 m-auto flex size-12 scale-90 items-center justify-center rounded-full border border-white/20 bg-black/40 text-white opacity-0 backdrop-blur-md transition-[opacity,transform] duration-[var(--dur-med)] group-hover:scale-100 group-hover:opacity-100 [&_svg]:size-5">
            <Play weight="fill" />
          </span>
        )}
        {working && (
          <div className="absolute inset-x-3 bottom-3">
            <Progress value={job.progress} className="bg-white/20" />
          </div>
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
  )
}
