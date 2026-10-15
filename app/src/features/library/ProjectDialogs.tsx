import { useState } from "react"
import { useNavigate, useRouterState } from "@tanstack/react-router"
import { CircleNotch } from "@/components/icons"
import { toast } from "sonner"
import type { ProjectSummary } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { commands, errorMessage } from "@/lib/ipc"

export function RenameProjectDialog({
  project,
  onOpenChange,
}: {
  project: ProjectSummary | null
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={project != null} onOpenChange={onOpenChange}>
      {project && <RenameForm project={project} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

function RenameForm({ project, onDone }: { project: ProjectSummary; onDone: () => void }) {
  const [title, setTitle] = useState(project.title)
  const save = async () => {
    try {
      await commands.projectRename(project.id, title)
      onDone()
    } catch (e) {
      toast.error("Couldn't rename the project", { description: errorMessage(e) })
    }
  }
  return (
    <DialogContent title="Rename project" description="The project folder on disk keeps its name.">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
      >
        <Field label="Title" htmlFor="rename-title">
          <Input id="rename-title" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        <DialogFooter>
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!title.trim() || title.trim() === project.title}>
            Rename
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
          <span className="block text-sm text-fg-1">Delete project files</span>
          <span className="block text-xs text-fg-3">The analysis, posters and previews in the .autoscene folder.</span>
        </span>
        <Switch checked={deleteFiles} onCheckedChange={setDeleteFiles} />
      </label>
      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button variant="danger" disabled={busy} onClick={() => void remove()}>
          {busy && <CircleNotch className="animate-spin" />}
          Remove project
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
