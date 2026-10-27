import { toast } from "sonner"
import type { ProjectSummary } from "@/bindings"
import { CONTENT_LABEL } from "./footage"
import { commands, errorMessage } from "./ipc"

/** "Series · Live action · 3 episodes". */
export function projectFacts(p: ProjectSummary): string {
  const unit = p.kind === "movie" ? (p.mediaCount === 1 ? "file" : "files") : p.mediaCount === 1 ? "episode" : "episodes"
  return [p.kind === "movie" ? "Movie" : "Series", CONTENT_LABEL[p.content], `${p.mediaCount} ${unit}`].join(" · ")
}

/** Add files or a folder to a project, saying how many were new. */
export async function addToProject(pid: string, paths: string[]) {
  if (!paths.length) return
  try {
    const n = await commands.mediaAdd(pid, paths)
    toast(n === 0 ? "Those files are already in this project" : `Added ${n} ${n === 1 ? "file" : "files"}`)
  } catch (e) {
    toast.error("Couldn't add files", { description: errorMessage(e) })
  }
}
