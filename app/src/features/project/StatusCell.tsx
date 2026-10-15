import { ArrowCounterClockwise, Check, Clock, FileX, Scissors, UserFocus, Warning } from "@/components/icons"
import { toast } from "sonner"
import type { JobInfo, MediaItem } from "@/bindings"
import { Progress } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { formatPercent } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { useMediaJob } from "@/state/activity"

function runningLabel(job: JobInfo): string {
  if (job.kind === "proxy") return "Preparing preview"
  if (job.kind === "analyze") return job.stage ?? "Analysing"
  return job.stage ?? "Indexing"
}

function queuedLabel(m: MediaItem, job?: JobInfo): string {
  if (m.status !== "ready") return "Queued"
  if (job?.kind === "proxy") return "Preview queued"
  if (job?.kind === "analyze") return "Analysis queued"
  return "Queued"
}

/** Indexing, preview and analysis state of one media, with live progress from its job. */
export function StatusCell({ media: m, projectId, faces }: { media: MediaItem; projectId: string; faces: boolean }) {
  const job = useMediaJob(m.id)
  const retry = (e: React.MouseEvent) => {
    e.stopPropagation()
    commands.mediaRetry(projectId, m.id).catch((err) => toast.error("Couldn't retry", { description: errorMessage(err) }))
  }
  const analyze = (e: React.MouseEvent) => {
    e.stopPropagation()
    commands
      .mediaAnalyze(projectId, m.id, null)
      .catch((err) => toast.error("Couldn't start the analysis", { description: errorMessage(err) }))
  }

  if (m.status === "missing") {
    return (
      <Tooltip label={m.path}>
        <span className="inline-flex items-center gap-1.5 text-sm text-danger">
          <FileX className="size-3.5" /> File missing
        </span>
      </Tooltip>
    )
  }
  if (m.status === "failed") {
    return (
      <span className="inline-flex items-center gap-2 text-sm">
        <Tooltip label={m.error ?? "Indexing failed"}>
          <span className="inline-flex items-center gap-1.5 text-danger">
            <Warning className="size-3.5" /> Failed
          </span>
        </Tooltip>
        <button onClick={retry} className="inline-flex items-center gap-1 rounded-xs px-1 text-fg-3 hover:text-fg-1">
          <ArrowCounterClockwise className="size-3" /> Retry
        </button>
      </span>
    )
  }
  if (job && job.state === "running") {
    return (
      <div className="flex w-40 flex-col gap-1.5">
        <div className="flex justify-between text-xs text-fg-3">
          <span className="truncate">{runningLabel(job)}</span>
          <span className="timecode">{formatPercent(job.progress)}</span>
        </div>
        <Progress value={job.progress} />
      </div>
    )
  }
  if (m.status !== "ready" || job) {
    return (
      <span className="inline-flex items-center gap-1.5 text-sm text-fg-3">
        <Clock className="size-3.5" /> {queuedLabel(m, job)}
      </span>
    )
  }
  if (m.shots === "failed") {
    return (
      <span className="inline-flex items-center gap-2 text-sm">
        <Tooltip label={m.shotsError ?? "Shot detection failed"}>
          <span className="inline-flex items-center gap-1.5 text-danger">
            <Warning className="size-3.5" /> Analysis failed
          </span>
        </Tooltip>
        <button onClick={analyze} className="inline-flex items-center gap-1 rounded-xs px-1 text-fg-3 hover:text-fg-1">
          <ArrowCounterClockwise className="size-3" /> Retry
        </button>
      </span>
    )
  }
  if (m.shots !== "ready") {
    return (
      <span className="inline-flex items-center gap-2 text-sm">
        <span className="inline-flex items-center gap-1.5 text-fg-2">
          <Check className="size-3.5 text-fg-3" /> {m.shots === "stopped" ? "Stopped" : "Indexed"}
        </span>
        <button onClick={analyze} className="inline-flex items-center gap-1 rounded-xs px-1 text-fg-3 hover:text-fg-1">
          <Scissors className="size-3" /> {m.shots === "stopped" ? "Continue" : "Find shots"}
        </button>
      </span>
    )
  }
  const shots = (
    <span className="inline-flex items-center gap-1.5 text-fg-2">
      <Check className="size-3.5 text-fg-3" /> <span className="timecode">{(m.shotCount ?? 0).toLocaleString("en-GB")}</span> shots
    </span>
  )
  if (!faces || m.faces === "ready") {
    return (
      <span className="inline-flex flex-col text-sm">
        {shots}
        {faces && (
          <span className="pl-5 text-xs text-fg-3">
            <span className="timecode">{(m.faceCount ?? 0).toLocaleString("en-GB")}</span> faces
          </span>
        )}
      </span>
    )
  }
  if (m.faces === "failed") {
    return (
      <span className="inline-flex flex-col gap-0.5 text-sm">
        {shots}
        <span className="inline-flex items-center gap-2 pl-5 text-xs">
          <Tooltip label={m.facesError ?? "Finding characters failed"}>
            <span className="inline-flex items-center gap-1 text-danger">
              <Warning className="size-3" /> Characters failed
            </span>
          </Tooltip>
          <button onClick={analyze} className="inline-flex items-center gap-1 rounded-xs px-1 text-fg-3 hover:text-fg-1">
            <ArrowCounterClockwise className="size-3" /> Retry
          </button>
        </span>
      </span>
    )
  }
  return (
    <span className="inline-flex flex-col gap-0.5 text-sm">
      {shots}
      <button onClick={analyze} className="inline-flex items-center gap-1 rounded-xs pl-5 text-left text-xs text-fg-3 hover:text-fg-1">
        <UserFocus className="size-3" /> {m.faces === "stopped" ? "Stopped · continue" : "Find characters"}
      </button>
    </span>
  )
}
