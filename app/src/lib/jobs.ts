import { toast } from "sonner"
import type { JobInfo } from "@/bindings"
import { commands, errorMessage } from "./ipc"

/**
 * Stopping a media's job, named by what that job does: a preview being
 * built is never stopped as "analysis".
 */
export function cancelLabel(kind: JobInfo["kind"]): string {
  switch (kind) {
    case "ingest":
      return "Cancel indexing"
    case "analyze":
      return "Cancel analysis"
    case "proxy":
      return "Cancel preview"
    default:
      return stopWord(kind)
  }
}

/** The word for stopping a job beside its title: a plugin's run is stopped, other work cancelled. */
export function stopWord(kind: JobInfo["kind"]): "Stop" | "Cancel" {
  return kind === "plugin" ? "Stop" : "Cancel"
}

/** Stop `job`, as the Activity drawer does; a toast says so when it can't. */
export function cancelJob(job: JobInfo) {
  const stop = stopWord(job.kind).toLowerCase()
  void commands.jobCancel(job.id).catch((e) => toast.error(`Couldn't ${stop} the job`, { description: errorMessage(e) }))
}
