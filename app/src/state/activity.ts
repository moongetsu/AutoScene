import { create } from "zustand"
import type { JobId, JobInfo, JobProgress } from "@/bindings"

/**
 * Live view of background jobs. Filled from `jobs_list` once and then kept
 * current by `job:changed` / `job:progress` events (see `lib/events.ts`).
 * Progress lives here, never in React Query, so 4 Hz updates don't refetch.
 */
type ActivityState = {
  jobs: Record<JobId, JobInfo>
  /** Newest first. */
  order: JobId[]
  replaceAll: (jobs: JobInfo[]) => void
  upsert: (job: JobInfo) => void
  progress: (p: JobProgress) => void
  clearFinished: () => void
}

export const isFinished = (j: JobInfo) => j.state === "done" || j.state === "failed" || j.state === "cancelled"

/**
 * Events and the initial snapshot can arrive in any order, so every write
 * keeps whichever copy of a job has the higher `rev` (see `JobInfo.rev`).
 */
export const useActivity = create<ActivityState>((set) => ({
  jobs: {},
  order: [],
  replaceAll: (list) =>
    set((s) => {
      const jobs: Record<JobId, JobInfo> = { ...s.jobs }
      for (const j of list) if (!jobs[j.id] || jobs[j.id].rev <= j.rev) jobs[j.id] = j
      const order = Object.values(jobs)
        .sort((a, b) => b.createdAt - a.createdAt)
        .map((j) => j.id)
      return { jobs, order }
    }),
  upsert: (job) =>
    set((s) => {
      const prev = s.jobs[job.id]
      if (prev && prev.rev > job.rev) return s
      return { jobs: { ...s.jobs, [job.id]: job }, order: prev ? s.order : [job.id, ...s.order] }
    }),
  progress: (p) =>
    set((s) => {
      const j = s.jobs[p.id]
      if (!j || isFinished(j) || j.rev > p.rev) return s
      const next = { ...j, progress: p.progress, stage: p.stage, etaMs: p.etaMs, frames: p.frames ?? null, rev: p.rev }
      return { jobs: { ...s.jobs, [p.id]: next } }
    }),
  clearFinished: () =>
    set((s) => {
      const keep = s.order.filter((id) => !isFinished(s.jobs[id]))
      return { order: keep, jobs: Object.fromEntries(keep.map((id) => [id, s.jobs[id]])) }
    }),
}))

/** All jobs, newest first. */
export function useJobs(): JobInfo[] {
  const jobs = useActivity((s) => s.jobs)
  const order = useActivity((s) => s.order)
  return order.map((id) => jobs[id]).filter(Boolean)
}

/** The unfinished job working on `mediaId` (of `kind`, if given), if any. */
export function useMediaJob(mediaId: string | undefined, kind?: JobInfo["kind"]): JobInfo | undefined {
  return useActivity((s) => {
    if (!mediaId) return undefined
    for (const id of s.order) {
      const j = s.jobs[id]
      if (j.mediaId === mediaId && !isFinished(j) && (!kind || j.kind === kind)) return j
    }
    return undefined
  })
}

/** Count of queued + running jobs (the sidebar's Activity badge). */
export function useActiveCount(): number {
  return useActivity((s) => s.order.reduce((n, id) => n + (isFinished(s.jobs[id]) ? 0 : 1), 0))
}

/** The mean progress of the running jobs `of` takes that report one (`null`: none does yet). */
function meanProgress(s: ActivityState, of: (j: JobInfo) => boolean): number | null {
  let [sum, n] = [0, 0]
  for (const id of s.order) {
    const j = s.jobs[id]
    if (j.state === "running" && j.progress != null && of(j)) {
      sum += j.progress
      n += 1
    }
  }
  return n > 0 ? sum / n : null
}

/**
 * How far running work is, as one number: the mean progress of the running
 * jobs that report one (`null`: nothing reports progress yet).
 */
export function useActiveProgress(): number | null {
  return useActivity((s) => meanProgress(s, () => true))
}

/** How far the indexing of `projectId` running now is (`null`: none reports progress yet). */
export function useProjectIndexProgress(projectId: string): number | null {
  return useActivity((s) => meanProgress(s, (j) => j.projectId === projectId && j.kind === "ingest"))
}

/** Is any media of `projectId` being indexed right now? */
export function useProjectIndexing(projectId: string): boolean {
  return useActivity((s) =>
    s.order.some((id) => {
      const j = s.jobs[id]
      return j.projectId === projectId && j.kind === "ingest" && !isFinished(j)
    }),
  )
}
