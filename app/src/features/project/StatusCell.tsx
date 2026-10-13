import { AlertTriangle, Check, Clock, FileQuestion, RotateCcw } from "lucide-react"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
import { Progress } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { formatPercent } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { useMediaJob } from "@/state/activity"

/** Indexing/preview state of one media, with live progress from its job. */
export function StatusCell({ media: m, projectId }: { media: MediaItem; projectId: string }) {
  const job = useMediaJob(m.id)
  const retry = (e: React.MouseEvent) => {
    e.stopPropagation()
    commands.mediaRetry(projectId, m.id).catch((err) => toast.error("Couldn't retry", { description: errorMessage(err) }))
  }

  if (m.status === "missing") {
    return (
      <Tooltip label={m.path}>
        <span className="inline-flex items-center gap-1.5 text-xs text-danger">
          <FileQuestion className="size-3.5" /> File missing
        </span>
      </Tooltip>
    )
  }
  if (m.status === "failed") {
    return (
      <span className="inline-flex items-center gap-2 text-xs">
        <Tooltip label={m.error ?? "Indexing failed"}>
          <span className="inline-flex items-center gap-1.5 text-danger">
            <AlertTriangle className="size-3.5" /> Failed
          </span>
        </Tooltip>
        <button onClick={retry} className="inline-flex items-center gap-1 rounded-xs px-1 text-fg-3 hover:text-fg-1">
          <RotateCcw className="size-3" /> Retry
        </button>
      </span>
    )
  }
  if (job && job.state === "running") {
    return (
      <div className="flex w-36 flex-col gap-1">
        <div className="flex justify-between text-2xs text-fg-3">
          <span className="truncate">{job.kind === "proxy" ? "Preparing preview" : (job.stage ?? "Indexing")}</span>
          <span className="timecode">{formatPercent(job.progress)}</span>
        </div>
        <Progress value={job.progress} />
      </div>
    )
  }
  if (m.status !== "ready" || job) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs text-fg-3">
        <Clock className="size-3.5" /> {m.status === "ready" ? "Preview queued" : "Queued"}
      </span>
    )
  }
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-fg-2">
      <Check className="size-3.5 text-fg-3" /> Ready
    </span>
  )
}
