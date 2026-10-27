import { useState } from "react"
import { useNavigate, useRouterState } from "@tanstack/react-router"
import { CircleNotch } from "@/components/icons"
import { toast } from "sonner"
import type { ContentKind, ProjectSummary } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { footageBlurb } from "@/lib/footage"
import { commands, errorMessage } from "@/lib/ipc"
import { FootagePicker } from "./FootagePicker"

/** A project's title and what its footage is. */
export function ProjectDetailsDialog({
  project,
  onOpenChange,
}: {
  project: ProjectSummary | null
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={project != null} onOpenChange={onOpenChange}>
      {project && <DetailsForm project={project} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

/** Anime is analysed differently from the start (see `Project::set_content`). */
const ANIME_APART = "Anime is analysed differently from the start. Add the videos as a new anime project instead."

function DetailsForm({ project, onDone }: { project: ProjectSummary; onDone: () => void }) {
  const [title, setTitle] = useState(project.title)
  const [content, setContent] = useState<ContentKind>(project.content)
  const [busy, setBusy] = useState(false)
  const retitled = title.trim() !== project.title
  const refilmed = content !== project.content
  // 3D animation is searched for faces harder (see `Project::faces_plan`).
  const researched = refilmed && (content === "animation" || project.content === "animation")
  const locked: Partial<Record<ContentKind, string>> =
    project.content === "anime"
      ? { live: ANIME_APART, animation: ANIME_APART, mixed: ANIME_APART }
      : { anime: ANIME_APART }
  const save = async () => {
    setBusy(true)
    // The footage first: it is the one that can be refused.
    let refilmedNow = false
    try {
      if (refilmed) {
        await commands.projectSetContent(project.id, content)
        refilmedNow = true
      }
      if (retitled) await commands.projectRename(project.id, title)
      onDone()
    } catch (e) {
      const what = refilmedNow ? "The footage changed, but the project couldn't be renamed" : "Couldn't change the project"
      toast.error(what, { description: errorMessage(e) })
      if (refilmedNow) onDone()
    } finally {
      setBusy(false)
    }
  }
  return (
    <DialogContent title="Project details" description="The project folder on disk keeps its name.">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
      >
        <Field label="Title" htmlFor="details-title">
          <Input id="details-title" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        <Field
          label="Footage"
          hint={
            researched
              ? "Faces are found again for it, then the characters regrouped. Your names and decisions stay."
              : refilmed
                ? "The characters are grouped again for it. Your names and decisions stay."
                : footageBlurb(content)
          }
          className="mt-4 items-start"
        >
          <FootagePicker value={content} onChange={setContent} locked={locked} />
        </Field>
        <DialogFooter>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={busy || !title.trim() || !(retitled || refilmed)}>
            {busy && <CircleNotch className="animate-spin" />}
            Save
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}

export function DeleteProjectDialog({
  project,
  onOpenChange,
}: {
  project: ProjectSummary | null
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={project != null} onOpenChange={onOpenChange}>
      {project && <DeleteForm project={project} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

function DeleteForm({ project, onDone }: { project: ProjectSummary; onDone: () => void }) {
  const [deleteFiles, setDeleteFiles] = useState(true)
  const [busy, setBusy] = useState(false)
  const navigate = useNavigate()
  const inProject = useRouterState({ select: (s) => s.location.pathname.startsWith(`/p/${project.id}`) })
  const remove = async () => {
    setBusy(true)
    try {
      await commands.projectDelete(project.id, deleteFiles)
      onDone()
      if (inProject) void navigate({ to: "/" })
      toast(`Removed “${project.title}”`)
    } catch (e) {
      toast.error("Couldn't remove the project", { description: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <DialogContent
      title={`Remove “${project.title}”?`}
      description="Your source videos and exported clips are never touched."
    >
      <label className="flex items-center justify-between gap-4 rounded-lg border border-hairline bg-inset px-3.5 py-3">
        <span>
          <span className="block text-sm text-fg-1">Delete its analysis too</span>
          <span className="block text-xs text-fg-3">
            Shots, faces, the cast with your names and decisions, posters and previews (the .autoscene folder). Added
            again later, the videos are analysed from scratch.
          </span>
        </span>
        <Switch checked={deleteFiles} onCheckedChange={setDeleteFiles} />
      </label>
      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button variant="danger" disabled={busy} onClick={() => void remove()}>
          {busy && <CircleNotch className="animate-spin" />}
          {deleteFiles ? "Delete project" : "Remove from library"}
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
