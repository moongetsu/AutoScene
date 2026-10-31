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
      return "Cancel"
  }
}

/** Stop `job`, as the Activity drawer does; a toast says so when it can't. */
export function cancelJob(job: JobInfo) {
  void commands.jobCancel(job.id).catch((e) => toast.error("Couldn't cancel the job", { description: errorMessage(e) }))
}
