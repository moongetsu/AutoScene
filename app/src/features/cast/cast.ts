import { useMemo } from "react"
import { toast } from "sonner"
import type { Cast, CastEdit, Character, JobInfo } from "@/bindings"
import { commands, errorMessage } from "@/lib/ipc"
import { useActivity, isFinished } from "@/state/activity"

/** Apply a cast decision, with a toast when it fails (then `undefined`). */
export async function castEdit(pid: string, edit: CastEdit): Promise<string | null | undefined> {
  try {
    return await commands.castEdit(pid, edit)
  } catch (e) {
    toast.error("Couldn't change the cast", { description: errorMessage(e) })
    return undefined
  }
}

/** Screen time as people say it: "42 s", "12 min", "1 h 05 min". */
export function formatScreenTime(sec: number): string {
  if (!Number.isFinite(sec) || sec <= 0) return "0 s"
  if (sec < 60) return `${Math.round(sec)} s`
  const m = Math.round(sec / 60)
  if (m < 60) return `${m} min`
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} min`
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString("en-GB")} ${n === 1 ? one : many}`
}

/** Visible characters first (as the backend sorts them), hidden ones apart. */
export function splitHidden(cast: Cast | undefined): { shown: Character[]; hidden: Character[] } {
  const all = cast?.characters ?? []
  return { shown: all.filter((c) => !c.hidden), hidden: all.filter((c) => c.hidden) }
}

/** The analysis jobs of a project that are queued or running, oldest first. */
export function useProjectAnalysis(pid: string): JobInfo[] {
  const jobs = useActivity((s) => s.jobs)
  const order = useActivity((s) => s.order)
  return useMemo(
    () =>
      order
        .map((id) => jobs[id])
        .filter((j) => j && j.kind === "analyze" && j.projectId === pid && !isFinished(j))
        .reverse(),
    [jobs, order, pid],
  )
}
