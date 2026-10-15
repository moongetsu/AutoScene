import { useState } from "react"
import { CircleNotch } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { commands, errorMessage } from "@/lib/ipc"
import { releaseTitle } from "@/lib/media"

/**
 * Ask before a file leaves the project: its shots, faces and the decisions
 * made about those faces go with it, and can't be brought back by adding the
 * file again (that analyses it from scratch).
 */
export function RemoveMediaDialog({
  projectId,
  media,
  onOpenChange,
}: {
  projectId: string
  media: MediaItem | null
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={media != null} onOpenChange={onOpenChange}>
      {media && <RemoveForm projectId={projectId} media={media} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

function RemoveForm({ projectId, media: m, onDone }: { projectId: string; media: MediaItem; onDone: () => void }) {
  const [busy, setBusy] = useState(false)
  const name = m.label ? `${m.label} · ${releaseTitle(m.fileName)}` : releaseTitle(m.fileName)
  const remove = async () => {
    setBusy(true)
    try {
      await commands.mediaRemove(projectId, m.id)
      onDone()
    } catch (e) {
      toast.error("Couldn't remove it", { description: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <DialogContent
      title={`Remove “${name}”?`}
      description="Its shots and faces go, and with them your decisions about those faces. Characters you haven't named who appear only here leave the cast. The video file itself isn't touched."
    >
      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button variant="danger" disabled={busy} onClick={() => void remove()}>
          {busy && <CircleNotch className="animate-spin" />}
          Remove from project
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
