import { toast } from "sonner"
import type { JobInfo } from "@/bindings"
import { useActivity } from "@/state/activity"
import { commands, events } from "./ipc"
import { qk, queryClient } from "./queries"

/**
 * Backend events → React Query invalidation and the activity store. The UI
 * never polls: every change the backend makes is announced here.
 *
 * | event            | effect                                             |
 * |------------------|----------------------------------------------------|
 * | job:changed      | activity store; toast when an export finishes      |
 * | job:progress     | activity store (progress only)                     |
 * | library:changed  | projects, project(id), exportDir                   |
 * | media:changed    | media(pid), frameMap(pid, mid), project(pid)       |
 * | export:changed   | exports(pid)                                       |
 */
export function startEventBridge(): () => void {
  const unlisten: Promise<() => void>[] = []

  unlisten.push(
    events.jobChanged.listen(({ payload }) => {
      const prev = useActivity.getState().jobs[payload.id]
      if (prev && prev.rev > payload.rev) return
      useActivity.getState().upsert(payload)
      if (prev?.state !== payload.state) notifyFinished(payload)
    }),
    events.jobProgress.listen(({ payload }) => useActivity.getState().progress(payload)),
    events.libraryChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.projects })
      if (payload.projectId) void queryClient.invalidateQueries({ queryKey: qk.project(payload.projectId) })
      // A rename moves the project's default export folder.
      void queryClient.invalidateQueries({ queryKey: ["exportDir"] })
    }),
    events.mediaChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.media(payload.projectId) })
      void queryClient.invalidateQueries({ queryKey: qk.project(payload.projectId) })
      const key = payload.mediaId
        ? qk.frameMap(payload.projectId, payload.mediaId)
        : (["frameMap", payload.projectId] as const)
      void queryClient.invalidateQueries({ queryKey: key })
    }),
    events.exportChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.exports(payload.projectId) })
    }),
  )
  // Snapshot only once the listeners are live, so nothing falls between the two.
  void Promise.all(unlisten)
    .then(() => commands.jobsList())
    .then((jobs) => useActivity.getState().replaceAll(jobs))

  return () => unlisten.forEach((p) => void p.then((un) => un()))
}

/** Exports finish in the background; say so wherever the user is. */
function notifyFinished(job: JobInfo) {
  if (job.kind !== "export") {
    if (job.state === "failed") toast.error(job.title.replace(/^Indexing/, "Couldn't index"), { description: job.error ?? undefined })
    return
  }
  if (job.state === "done") toast.success("Export finished", { description: job.subtitle ?? undefined })
  else if (job.state === "failed") toast.error("Export finished with errors", { description: job.error ?? undefined })
}
