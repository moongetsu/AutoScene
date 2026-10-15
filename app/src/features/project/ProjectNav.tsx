import { useRouterState } from "@tanstack/react-router"
import { SegmentedNav } from "@/components/ui/segmented"
import { useProject } from "@/lib/queries"

/** A project's sections, in the middle of its top bar. */
export function ProjectNav({ pid }: { pid: string }) {
  const { data: project } = useProject(pid)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const base = `/p/${pid}`
  const items = [
    { to: base, label: "Overview", active: path === base || path === `${base}/` },
    {
      to: `${base}/episodes`,
      label: project?.kind === "movie" ? "Files" : "Episodes",
      active: path === `${base}/episodes`,
    },
    { to: `${base}/cast`, label: "Cast", active: path.startsWith(`${base}/cast`) },
    { to: `${base}/exports`, label: "Exports", active: path === `${base}/exports` },
  ]
  return <SegmentedNav aria-label="Project" items={items} />
}
