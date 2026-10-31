import { toast } from "sonner"
import type { JobInfo } from "@/bindings"
import { isFinished, useActivity } from "@/state/activity"
import { commands, events } from "./ipc"
import { reveal } from "./os"
import { exportsQuery, qk, queryClient } from "./queries"

/**
 * Backend events → React Query invalidation and the activity store. The UI
 * never polls: every change the backend makes is announced here.
 *
 * | event            | effect                                             |
 * |------------------|----------------------------------------------------|
 * | job:changed      | activity store; toast when an export finishes, exports(pid) when it ends |
 * | job:progress     | activity store (progress only)                     |
 * | library:changed  | projects, project(id), exportDir                   |
 * | media:changed    | media(pid), frameMap(pid, mid), project(pid)       |
 * | analysis:changed | segments(pid, mid)                                 |
 * | cast:changed     | cast(pid) and everything under it (tracks, presence, packs) |
 * | pack:changed     | pack(pid, cid): its settings (an export keeps them) |
 * | export:changed   | exports(pid)                                       |
 */
export function startEventBridge(): () => void {
  const unlisten: Promise<() => void>[] = []

  unlisten.push(
    events.jobChanged.listen(({ payload }) => {
      const prev = useActivity.getState().jobs[payload.id]
      if (prev && prev.rev > payload.rev) return
      useActivity.getState().upsert(payload)
      if (prev?.state === payload.state) return
      // An export's card reads it again when its job ends, should the
      // export's own event be late or not come (cancelled while queued).
      if (payload.kind === "export" && payload.projectId && isFinished(payload)) {
        void queryClient.invalidateQueries({ queryKey: qk.exports(payload.projectId) })
      }
      notifyFinished(payload)
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
      // A file gone or back, indexed again: the packs cut from it change.
      void queryClient.invalidateQueries({ queryKey: [...qk.cast(payload.projectId), "pack"] })
      const key = payload.mediaId
        ? qk.frameMap(payload.projectId, payload.mediaId)
        : (["frameMap", payload.projectId] as const)
      void queryClient.invalidateQueries({ queryKey: key })
    }),
    events.analysisChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.segments(payload.projectId, payload.mediaId) })
    }),
    events.castChanged.listen(({ payload }) => {
      // Prefix match: the cast, its track lists, presence lanes and clip lists.
      void queryClient.invalidateQueries({ queryKey: qk.cast(payload.projectId) })
      // Portraits may have been made anew: they are asked for again when
      // next shown, not now (the face on screen stays).
      void queryClient.invalidateQueries({ queryKey: qk.portraits(payload.projectId), refetchType: "none" })
    }),
    events.anilistChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.anilistNames(payload.projectId), exact: true })
    }),
    events.portraitsChanged.listen(({ payload }) => {
      // Covers got sharp: the cast alone, not its lists.
      void queryClient.invalidateQueries({ queryKey: qk.cast(payload.projectId), exact: true })
    }),
    events.packChanged.listen(({ payload }) => {
      void queryClient.invalidateQueries({ queryKey: qk.pack(payload.projectId, payload.characterId) })
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
    if (job.state === "failed") {
      const title = job.title.replace(/^Indexing/, "Couldn't index").replace(/^Analysing/, "Couldn't analyse")
      toast.error(title, { description: job.error ?? undefined })
    }
    return
  }
  if (job.state === "done") {
    // Said once it's known where the export went (a moment): Show in folder goes there.
    void exportPlace(job).then((place) =>
      toast.success("Export finished", {
        description: job.subtitle ?? undefined,
        action: place ? { label: "Show in folder", onClick: () => reveal(place) } : undefined,
      }),
    )
  } else if (job.state === "failed") toast.error("Export finished with errors", { description: job.error ?? undefined })
}

/**
 * Where a finished export is: its one file (the clips joined, or a single
 * clip), else its folder. `null` when that can't be read.
 */
async function exportPlace(job: JobInfo): Promise<string | null> {
  if (!job.projectId || !job.exportId) return null
  try {
    // Read afresh: the list on hand may be from before the export was made.
    const list = await queryClient.fetchQuery({ ...exportsQuery(job.projectId), staleTime: 0 })
    const ex = list.find((e) => e.id === job.exportId)
    if (!ex) return null
    const single = ex.items.length === 1 ? ex.items[0].path : null
    return ex.merged?.path ?? single ?? ex.outDir
  } catch {
    return null
  }
}
