import { mediaTitle } from "@/lib/media"
import { useMediaList, useProject } from "@/lib/queries"

/** Title-bar breadcrumbs that follow renames live. */
export function ProjectCrumb({ pid }: { pid: string }) {
  const { data } = useProject(pid)
  return <>{data?.title ?? "Project"}</>
}

export function MediaCrumb({ pid, mid }: { pid: string; mid: string }) {
  const { data } = useMediaList(pid)
  const m = data?.find((x) => x.id === mid)
  return <>{m ? mediaTitle(m) : "Episode"}</>
}
