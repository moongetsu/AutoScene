import { useRef, useState } from "react"
import { ImagePlus, Loader2 } from "lucide-react"
import { toast } from "sonner"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type CastPackDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  projectId: string
  onDone: () => Promise<void> | void
}

export function CastPackDialog({
  open,
  onOpenChange,
  projectId,
  onDone,
}: CastPackDialogProps) {
  const inputRef = useRef<HTMLInputElement | null>(null)
  const [busy, setBusy] = useState(false)
  const [dragOver, setDragOver] = useState(false)

  const run = async (files: FileList | File[] | null) => {
    const list = files ? [...files].filter((f) => f.type.startsWith("image/")) : []
    if (!list.length) {
      toast.message("Add images named like Ahsoka.jpg")
      return
    }
    setBusy(true)
    try {
      const res = await api.teachCast(projectId, list, { mode: "pack" })
      const linked = res.matches.filter((m) => m.status === "linked")
      const missed = res.matches.filter((m) => m.status === "unmatched")
      if (linked.length) {
        toast.success(
          `Named ${linked.length} character${linked.length === 1 ? "" : "s"} from photos`,
        )
      }
      if (missed.length) {
        toast.message(
          `${missed.length} unmatched (no face or below match floor)`,
        )
      }
      await onDone()
      onOpenChange(false)
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setBusy(false)
      if (inputRef.current) inputRef.current.value = ""
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Cast photos</DialogTitle>
          <DialogDescription>
            Drop stills named after characters (Ahsoka.png). AutoScene matches each face to the
            closest cast member and renames them.
          </DialogDescription>
        </DialogHeader>
        <div
          onDragOver={(e) => {
            e.preventDefault()
            setDragOver(true)
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={(e) => {
            e.preventDefault()
            setDragOver(false)
            if (!busy) void run(e.dataTransfer.files)
          }}
          className={cn(
            "flex min-h-[160px] flex-col items-center justify-center gap-2 rounded-xl border border-dashed px-4 py-8 text-center transition",
            dragOver
              ? "border-primary/50 bg-primary/10"
              : "border-border/50 bg-muted/20",
            busy && "opacity-70",
          )}
        >
          {busy ? (
            <Loader2 className="size-6 animate-spin text-primary" />
          ) : (
            <ImagePlus className="size-6 text-muted-foreground" />
          )}
          <p className="text-sm text-muted-foreground">
            {busy ? "Matching cast…" : "Drop reference stills here"}
          </p>
        </div>
        <input
          ref={inputRef}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={(e) => void run(e.target.files)}
        />
        <Button
          type="button"
          disabled={busy}
          className="w-full"
          onClick={() => inputRef.current?.click()}
        >
          Choose photos
        </Button>
      </DialogContent>
    </Dialog>
  )
}
