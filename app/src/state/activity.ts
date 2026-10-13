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

export const useActivity = create<ActivityState>((set) => ({
  jobs: {},
  order: [],
  replaceAll: (list) =>
    set({ jobs: Object.fromEntries(list.map((j) => [j.id, j])), order: list.map((j) => j.id) }),
  upsert: (job) =>
    set((s) => ({
      jobs: { ...s.jobs, [job.id]: job },
      order: s.jobs[job.id] ? s.order : [job.id, ...s.order],
    })),
  progress: (p) =>
    set((s) => {
      const j = s.jobs[p.id]
      if (!j || isFinished(j)) return s
      return { jobs: { ...s.jobs, [p.id]: { ...j, progress: p.progress, stage: p.stage, etaMs: p.etaMs } } }
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

/** The unfinished job working on `mediaId` (indexing or preview), if any. */
export function useMediaJob(mediaId: string | undefined): JobInfo | undefined {
  return useActivity((s) => {
    if (!mediaId) return undefined
    for (const id of s.order) {
      const j = s.jobs[id]
      if (j.mediaId === mediaId && !isFinished(j)) return j
    }
    return undefined
  })
}

/** Count of queued + running jobs (title bar badge). */
export function useActiveCount(): number {
  return useActivity((s) => s.order.reduce((n, id) => n + (isFinished(s.jobs[id]) ? 0 : 1), 0))
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
