import { useState } from "react"
import { Loader2, Trash2 } from "lucide-react"
import { toast } from "sonner"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { api, type Project } from "@/lib/api"
import { friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type DeleteProjectButtonProps = {
  project: Project
  onDeleted: (id: string) => void | Promise<void>
  /** Show trash + Delete label (easier to find than icon-only). */
  showLabel?: boolean
  className?: string
}

export function DeleteProjectButton({
  project,
  onDeleted,
  showLabel = false,
  className,
}: DeleteProjectButtonProps) {
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [ackStop, setAckStop] = useState(false)
  const scanBusy =
    project.status === "cutting" ||
    project.status === "scanning" ||
    project.status === "paused"
  const canDelete = !scanBusy || ackStop

  const handleDelete = async () => {
    if (!canDelete) return
    setBusy(true)
    try {
      // Always confirm_stop after dialog — covers race where scan is running
      // but UI status has not refreshed yet.
      await api.deleteProject(project.id, { confirmStop: true })
      toast.success(
        scanBusy
          ? `Stopped scan and deleted “${project.name}”`
          : `Deleted “${project.name}”`,
      )
      setOpen(false)
      await onDeleted(project.id)
    } catch (e) {
      toast.error(friendlyError(e, "Delete failed"))
    } finally {
      setBusy(false)
    }
  }

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (!next) {
          setAckStop(false)
          setBusy(false)
        }
      }}
    >
      <AlertDialogTrigger asChild>
        <Button
          type="button"
          variant={showLabel ? "outline" : "ghost"}
          size={showLabel ? "sm" : "icon-sm"}
          className={cn(
            "shrink-0",
            showLabel
              ? "h-8 gap-1.5 border-destructive/35 text-destructive hover:bg-destructive/10"
              : "text-muted-foreground hover:bg-destructive/10 hover:text-destructive",
            className,
          )}
          onClick={(e) => e.stopPropagation()}
          aria-label={`Delete ${project.name}`}
        >
          <Trash2 className="size-3.5" />
          {showLabel ? "Delete" : null}
        </Button>
      </AlertDialogTrigger>

      <AlertDialogContent
        size="default"
        className="max-w-md"
        onClick={(e) => e.stopPropagation()}
      >
        <AlertDialogHeader className="min-w-0">
          <AlertDialogMedia className="bg-destructive/15 text-destructive">
            <Trash2 />
          </AlertDialogMedia>
          <AlertDialogTitle>Delete project?</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div className="space-y-2 text-left">
              <p>Permanently remove this project:</p>
              <p className="break-all font-medium text-foreground [overflow-wrap:anywhere]">
                {project.name}
              </p>
              <p>
                Project folder, thumbs, and exports go with it. Source files outside the project
                folder stay on disk. No undo.
              </p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>

        {scanBusy ? (
          <label
            className={cn(
              "flex min-w-0 cursor-pointer items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-3 text-left",
            )}
          >
            <input
              type="checkbox"
              className="mt-0.5 size-4 shrink-0 accent-[oklch(0.78_0.14_65)]"
              checked={ackStop}
              disabled={busy}
              onChange={(e) => setAckStop(e.target.checked)}
            />
            <span className="min-w-0 flex-1 text-xs leading-relaxed break-words text-muted-foreground">
              Scan is {project.status === "paused" ? "paused" : "running"}. I agree AutoScene must{" "}
              <span className="font-medium text-foreground">fully stop the scan first</span>, then
              delete this project.
            </span>
          </label>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={!canDelete || busy}
            onClick={(e) => {
              e.preventDefault()
              void handleDelete()
            }}
          >
            {busy ? (
              <>
                <Loader2 className="size-3.5 animate-spin" />
                {scanBusy ? "Stopping & deleting…" : "Deleting…"}
              </>
            ) : scanBusy ? (
              "Stop scan & delete"
            ) : (
              "Delete forever"
            )}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
