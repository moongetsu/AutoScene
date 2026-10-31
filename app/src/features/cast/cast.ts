import { useMemo } from "react"
import { toast } from "sonner"
import type { Cast, CastEdit, Character, ContentKind, JobInfo, ProjectKind } from "@/bindings"
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

/** Where a project's character names come from, by its footage and
 *  whether it is a film (as the backend's `Source::of`): AniList for anime,
 *  TVmaze for an animated series, the film's Fandom wiki for an animated
 *  film, none for the rest. */
export function namesSource(
  content: ContentKind | undefined,
  kind: ProjectKind | undefined,
): "AniList" | "TVmaze" | "Fandom" | null {
  if (content === "anime") return "AniList"
  if (content === "animation") return kind === "movie" ? "Fandom" : "TVmaze"
  return null
}

/** Where a source's names and pictures come from, as its licence asks it
 *  to be said (`null`: nothing to say). */
export function namesCredit(source: ReturnType<typeof namesSource>): string | null {
  if (source === "TVmaze") return "Names and pictures: TVmaze.com (CC BY-SA)."
  if (source === "Fandom") return "Names and pictures: the film's Fandom wiki (CC BY-SA), found through Wikidata."
  return null
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

/**
 * What the cast says of the user's decisions whose faces a later analysis
 * didn't find again (`Cast.unplaced`): they wait and are tried again.
 * Nothing when there are none.
 */
export function unplacedNote(n: number): string | null {
  if (!(n > 0)) return null
  const count = n.toLocaleString("en-GB")
  return n === 1
    ? `${count} of your decisions waits for a face the latest analysis didn't find again. It's tried again at the next analysis.`
    : `${count} of your decisions wait for faces the latest analysis didn't find again. They're tried again at the next analysis.`
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
