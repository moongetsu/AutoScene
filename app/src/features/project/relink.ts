import { toast } from "sonner"
import type { MediaItem, Relink, RelinkSummary } from "@/bindings"
import { plural } from "@/features/cast/cast"
import { commands, errorMessage } from "@/lib/ipc"
import { mediaTitle, pickFolder, pickVideoFile } from "@/lib/media"
import { useRelink } from "@/state/relink"

/**
 * Show AutoScene where a missing episode is now. The same file keeps
 * everything; another copy is indexed again; another video is asked about
 * first (`RelinkDialog`).
 */
export async function locateFile(pid: string, m: MediaItem) {
  const path = await pickVideoFile(`Where is ${mediaTitle(m)} now?`)
  if (!path) return
  try {
    const r = await commands.mediaRelink(pid, m.id, path, false)
    if (r.kind === "different") useRelink.getState().ask({ pid, media: m, path, reason: r.reason })
    else relinked(r, mediaTitle(m))
  } catch (e) {
    toast.error("Couldn't use that file", { description: errorMessage(e) })
  }
}

/** The file asked about, taken anyway: analysed again, decisions kept. */
export async function relinkAnyway(pid: string, m: MediaItem, path: string) {
  try {
    await commands.mediaRelink(pid, m.id, path, true)
    toast(`Using it for ${mediaTitle(m)}`, { description: "It's analysed again; your decisions about its faces are kept." })
  } catch (e) {
    toast.error("Couldn't use that file", { description: errorMessage(e) })
  }
}

function relinked(r: Relink, title: string) {
  toast(
    r.kind === "same" ? `Found ${title}` : `Found ${title} — indexing it again`,
    r.kind === "same" ? undefined : { description: "Its analysis stays if its frames are the same." },
  )
}

/** Look for every missing episode in one folder (and the folders inside it). */
export async function locateFolder(pid: string, missing: number) {
  const dir = await pickFolder("Where are the episodes now?")
  if (!dir) return
  try {
    const s = await commands.mediaRelinkFolder(pid, dir)
    const { title, description } = folderResult(s, missing)
    if (s.same + s.reindex === 0) toast.error(title, { description })
    else toast(title, { description })
  } catch (e) {
    toast.error("Couldn't look in that folder", { description: errorMessage(e) })
  }
}

export function folderResult(s: RelinkSummary, missing: number): { title: string; description?: string } {
  const found = s.same + s.reindex
  const checking = s.reindex > 0 ? `${plural(s.reindex, "is another copy", "are other copies")}, indexed again.` : undefined
  if (found === 0) return { title: "None of them are in that folder", description: "Pick the folder the episodes are in now." }
  if (s.missing === 0) {
    return { title: found === 1 ? "Found the episode" : `Found all ${found} episodes`, description: checking }
  }
  const still = `${plural(s.missing, "episode")} still can't be found.`
  return { title: `Found ${found} of ${missing} episodes`, description: checking ? `${checking} ${still}` : still }
}
