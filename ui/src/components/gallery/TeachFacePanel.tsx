import { useRef, useState } from "react"
import { ImagePlus, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { Input } from "@/components/ui/input"
import { api } from "@/lib/api"
import { friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type TeachFacePanelProps = {
  projectId: string
  personId: number
  personLabel: string
  disabled?: boolean
  onTaught: () => Promise<void> | void
  className?: string
}

export function TeachFacePanel({
  projectId,
  personId,
  personLabel,
  disabled,
  onTaught,
  className,
}: TeachFacePanelProps) {
  const inputRef = useRef<HTMLInputElement | null>(null)
  const [busy, setBusy] = useState(false)
  const [label, setLabel] = useState(personLabel)
  const [dragOver, setDragOver] = useState(false)

  const run = async (files: FileList | File[] | null) => {
    const list = files ? [...files].filter((f) => f.type.startsWith("image/")) : []
    if (!list.length) {
      toast.message("Drop image files (jpg, png, webp)")
      return
    }
    setBusy(true)
    try {
      const res = await api.teachCast(projectId, list, {
        mode: "pin",
        personId,
        label: label.trim() || personLabel,
      })
      const ok = res.matches.filter((m) => m.status === "pinned" || m.status === "linked")
      const bad = res.matches.filter((m) => m.status === "unmatched")
      if (ok.length) {
        const sim = ok[0].similarity
        toast.success(
          `Linked to ${ok[0].label}${sim ? ` (${Math.round(sim * 100)}% match)` : ""}`,
        )
      }
      if (bad.length) toast.message(`${bad.length} image(s) had no face`)
      await onTaught()
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setBusy(false)
      if (inputRef.current) inputRef.current.value = ""
    }
  }

  return (
    <div
      className={cn(
        "min-w-0 max-w-full space-y-2 overflow-hidden border-t border-border/35 pt-3",
        className,
      )}
    >
      <div className="flex min-w-0 items-baseline justify-between gap-2">
        <p className="shrink-0 text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
          Teach face
        </p>
        <p className="truncate text-[10px] text-muted-foreground/80">
          Locks name for later scans
        </p>
      </div>
      <div className="flex min-w-0 gap-2">
        <Input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Character name"
          disabled={disabled || busy}
          className="h-9 min-w-0 flex-1 bg-background/40"
        />
        <button
          type="button"
          disabled={disabled || busy}
          onClick={() => inputRef.current?.click()}
          onDragOver={(e) => {
            e.preventDefault()
            setDragOver(true)
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={(e) => {
            e.preventDefault()
            setDragOver(false)
            if (!disabled && !busy) void run(e.dataTransfer.files)
          }}
          className={cn(
            "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md border px-3 text-xs transition",
            dragOver
              ? "border-primary/50 bg-primary/10 text-primary"
              : "border-border/50 bg-background/40 text-muted-foreground hover:border-border hover:text-foreground",
            (disabled || busy) && "pointer-events-none opacity-60",
          )}
        >
          {busy ? (
            <Loader2 className="size-3.5 animate-spin text-primary" />
          ) : (
            <ImagePlus className="size-3.5" />
          )}
          {busy ? "Matching" : "Still"}
        </button>
      </div>
      <input
        ref={inputRef}
        type="file"
        accept="image/*"
        multiple
        className="hidden"
        onChange={(e) => void run(e.target.files)}
      />
    </div>
  )
}
