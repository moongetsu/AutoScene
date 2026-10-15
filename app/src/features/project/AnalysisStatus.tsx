import { UserFocus } from "@/components/icons"
import { Progress } from "@/components/ui/progress"
import { plural, useProjectAnalysis } from "@/features/cast/cast"
import { formatPercent } from "@/lib/format"
import { useMediaList } from "@/lib/queries"
import { cn } from "@/lib/utils"

/** What the analysis of a project is doing right now (nothing when idle). */
export function AnalysisStatus({ pid, className }: { pid: string; className?: string }) {
  const jobs = useProjectAnalysis(pid)
  const { data: media } = useMediaList(pid)
  const job = jobs.find((j) => j.state === "running") ?? jobs[0]
  if (!job) return null
  const m = media?.find((x) => x.id === job.mediaId)
  const waiting = jobs.length - 1
  return (
    <div
      className={cn(
        "flex w-full max-w-md items-center gap-3.5 rounded-2xl border border-white/10 bg-black/35 px-4 py-3 backdrop-blur-xl",
        className,
      )}
    >
      <span className="relative flex size-9 shrink-0 items-center justify-center rounded-full bg-accent-subtle text-accent [&_svg]:size-[18px]">
        <span className="absolute inset-0 animate-ping rounded-full bg-accent/20 [animation-duration:2.4s]" />
        <UserFocus />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-3 text-sm">
          <span className="truncate font-medium text-fg-1">
            {job.state === "queued" ? "Waiting to analyse" : (job.stage ?? "Analysing")}
            {m && <span className="font-normal text-fg-2"> · {m.label || m.fileName}</span>}
          </span>
          <span className="timecode shrink-0 text-xs text-fg-2">{formatPercent(job.progress)}</span>
        </div>
        <Progress className="mt-2" value={job.state === "running" ? job.progress : null} />
        {waiting > 0 && <div className="mt-1.5 text-xs text-fg-3">{plural(waiting, "more episode")} queued</div>}
      </div>
    </div>
  )
}
