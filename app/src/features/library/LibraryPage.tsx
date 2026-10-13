import type * as React from "react"
import { useState } from "react"
import { Clapperboard, Plus, Scissors, Users } from "lucide-react"
import type { ProjectSummary } from "@/bindings"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { Button } from "@/components/ui/button"
import { Page, PageHeader } from "@/components/ui/page"
import { useProjects } from "@/lib/queries"
import { useNewProject } from "@/state/new-project"
import { ProjectCard } from "./ProjectCard"
import { DeleteProjectDialog, RenameProjectDialog } from "./ProjectDialogs"

const STEPS: { icon: React.ReactNode; title: string; body: string }[] = [
  {
    icon: <Clapperboard />,
    title: "Add episodes",
    body: "Drop a season folder or a movie. Episodes are recognised and put in order automatically.",
  },
  {
    icon: <Users />,
    title: "Review the cast",
    body: "Every character is found across all episodes. Rename, merge or correct them in a keystroke.",
  },
  {
    icon: <Scissors />,
    title: "Export packs",
    body: "Frame-exact clips in ProRes, DNxHR or H.264, with timelines for Premiere, Resolve and Final Cut.",
  },
]

export function LibraryPage() {
  const { data: projects, isPending } = useProjects()
  const newProject = useNewProject((s) => s.show)
  const [renaming, setRenaming] = useState<ProjectSummary | null>(null)
  const [deleting, setDeleting] = useState<ProjectSummary | null>(null)
  const empty = !isPending && (projects?.length ?? 0) === 0

  return (
    <Page>
      <PageHeader
        title="Library"
        description="Series and movies you've turned into scenepacks."
        actions={
          !empty && (
            <Button variant="primary" onClick={() => newProject()}>
              <Plus /> New project
            </Button>
          )
        }
      />
      <div className="px-8 pb-10">
        {empty ? (
          <Welcome onCreate={() => newProject()} />
        ) : (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(232px,1fr))] gap-5">
            {isPending
              ? Array.from({ length: 4 }, (_, i) => (
                  <div key={i} className="overflow-hidden rounded-lg border border-hairline bg-panel">
                    <div className="skeleton aspect-video" />
                    <div className="space-y-2 px-3.5 py-3">
                      <div className="skeleton h-3.5 w-2/3 rounded-xs" />
                      <div className="skeleton h-3 w-1/2 rounded-xs" />
                    </div>
                  </div>
                ))
              : projects!.map((p) => (
                  <ProjectCard key={p.id} project={p} onRename={() => setRenaming(p)} onDelete={() => setDeleting(p)} />
                ))}
          </div>
        )}
      </div>
      <RenameProjectDialog project={renaming} onOpenChange={(o) => !o && setRenaming(null)} />
      <DeleteProjectDialog project={deleting} onOpenChange={(o) => !o && setDeleting(null)} />
    </Page>
  )
}

function Welcome({ onCreate }: { onCreate: () => void }) {
  return (
    <section className="relative overflow-hidden rounded-xl border border-hairline bg-panel shadow-1">
      {/* Neutral graphite light — depth without a coloured glow. */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(70%_60%_at_50%_0%,rgb(255_255_255/0.05),transparent_70%)]"
      />
      <div className="relative flex flex-col items-center px-8 pt-14 pb-10 text-center">
        <AutoSceneMark className="size-14" />
        <h2 className="mt-6 text-3xl text-fg-1">Build your first scenepack</h2>
        <p className="mt-2 max-w-lg text-md text-fg-2">
          AutoScene finds every shot, learns the cast across episodes, and cuts clean, frame-exact clips ready for your
          edit.
        </p>
        <div className="mt-7 flex items-center gap-3">
          <Button variant="primary" size="lg" onClick={onCreate}>
            <Plus /> New project
          </Button>
          <span className="text-xs text-fg-3">or drop a season folder anywhere</span>
        </div>
      </div>
      <ol className="relative grid grid-cols-1 gap-px border-t border-hairline bg-hairline md:grid-cols-3">
        {STEPS.map((s, i) => (
          <li key={s.title} className="flex gap-3.5 bg-panel px-6 py-5">
            <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-hairline bg-raised text-fg-2 [&_svg]:size-4">
              {s.icon}
            </div>
            <div>
              <div className="flex items-baseline gap-2">
                <span className="timecode text-2xs text-fg-3">0{i + 1}</span>
                <h3 className="text-sm font-semibold text-fg-1">{s.title}</h3>
              </div>
              <p className="mt-1 text-xs leading-[18px] text-fg-2">{s.body}</p>
            </div>
          </li>
        ))}
      </ol>
    </section>
  )
}
