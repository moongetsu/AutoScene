import { mediaTitle } from "@/lib/media"
import { useCast, useMediaList, useProject } from "@/lib/queries"

/** Names of the views for the window title; they follow renames live. */
export function ProjectCrumb({ pid }: { pid: string }) {
  const { data } = useProject(pid)
  return <>{data?.title ?? "Project"}</>
}

export function MediaCrumb({ pid, mid }: { pid: string; mid: string }) {
  const { data } = useMediaList(pid)
  const m = data?.find((x) => x.id === mid)
  return <>{m ? mediaTitle(m) : "Episode"}</>
}

export function CharacterCrumb({ pid, cid }: { pid: string; cid: string }) {
  const { data } = useCast(pid)
  if (cid === "unassigned") return <>Cast · Everyone else</>
  const c = data?.characters.find((x) => x.id === cid)
  return <>Cast · {c?.label ?? "Character"}</>
}
