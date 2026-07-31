import { useEffect, useMemo, useState } from "react"
import { Check, GitMerge } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { FacePortrait } from "@/components/shared/FacePortrait"
import { api, appendQuery, formatDuration, type Person } from "@/lib/api"
import { friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type MergeCastDialogProps = {
  open: boolean
  onOpenChange: (open: boolean) => void
  projectId: string
  people: Person[]
  /** Survivor identity */
  keepPersonId: number
  /** Pre-checked absorb ids (optional) */
  initialMergeIds?: number[]
  cacheRev: string
  onMerged: (keepPersonId: number) => Promise<void>
}

export function MergeCastDialog({
  open,
  onOpenChange,
  projectId,
  people,
  keepPersonId,
  initialMergeIds = [],
  cacheRev,
  onMerged,
}: MergeCastDialogProps) {
  const keep = people.find((p) => p.person_id === keepPersonId) ?? null
  const others = useMemo(
    () => people.filter((p) => p.person_id !== keepPersonId),
    [people, keepPersonId],
  )
  const [selected, setSelected] = useState<Set<number>>(
    () => new Set(initialMergeIds.filter((id) => id !== keepPersonId)),
  )
  const [busy, setBusy] = useState(false)

  const initialKey = initialMergeIds.slice().sort((a, b) => a - b).join(",")
  useEffect(() => {
    if (!open) return
    setSelected(
      new Set(
        initialKey
          ? initialKey.split(",").map(Number).filter((id) => id !== keepPersonId)
          : [],
      ),
    )
  }, [open, keepPersonId, initialKey])

  const toggle = (id: number) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const confirm = async () => {
    if (!keep || selected.size === 0) return
    setBusy(true)
    try {
      await api.mergePeople(projectId, {
        keep_person_id: keepPersonId,
        merge_person_ids: [...selected],
      })
      await onMerged(keepPersonId)
      toast.success(
        `Merged ${selected.size} into ${keep.label}. Scenes kept in timeline order.`,
      )
      onOpenChange(false)
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg gap-0 overflow-hidden p-0 sm:max-w-xl">
        <DialogHeader className="border-b border-border/50 px-5 py-4 text-left">
          <DialogTitle className="font-heading text-xl">Merge into cast</DialogTitle>
          <DialogDescription>
            Absorb lookalikes into{" "}
            <span className="text-foreground">{keep?.label ?? "selected"}</span>. All
            scenes stay chronological. Cannot undo.
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[50vh] space-y-2 overflow-y-auto px-5 py-4">
          {keep ? (
            <div className="mb-3 flex items-center gap-3 rounded-xl border border-primary/35 bg-primary/5 px-3 py-2.5">
              <Thumb projectId={projectId} person={keep} cacheRev={cacheRev} />
              <div className="min-w-0">
                <p className="text-[10px] uppercase tracking-[0.16em] text-primary">Keep</p>
                <p className="truncate font-medium">{keep.label}</p>
                <p className="timecode text-xs text-muted-foreground">
                  {keep.detection_count} faces · {formatDuration(keep.screen_time_sec)}
                </p>
              </div>
            </div>
          ) : null}

          {others.length === 0 ? (
            <p className="text-sm text-muted-foreground">No other characters to merge.</p>
          ) : (
            others.map((person) => {
              const on = selected.has(person.person_id)
              return (
                <button
                  key={person.person_id}
                  type="button"
                  onClick={() => toggle(person.person_id)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition",
                    on
                      ? "border-amber-500/40 bg-amber-500/10"
                      : "border-border/40 bg-background/30 hover:border-border",
                  )}
                >
                  <span
                    className={cn(
                      "flex size-4 shrink-0 items-center justify-center rounded border",
                      on
                        ? "border-amber-400 bg-amber-500/30 text-amber-100"
                        : "border-border/60 text-transparent",
                    )}
                  >
                    {on ? <Check className="size-3" /> : null}
                  </span>
                  <Thumb projectId={projectId} person={person} cacheRev={cacheRev} />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{person.label}</p>
                    <p className="timecode text-[11px] text-muted-foreground">
                      {person.detection_count} faces · {person.clip_count} clips ·{" "}
                      {formatDuration(person.screen_time_sec)}
                    </p>
                  </div>
                </button>
              )
            })
          )}
        </div>

        <DialogFooter className="border-t border-border/50 px-5 py-4 sm:justify-between">
          <p className="text-xs text-muted-foreground">
            {selected.size} selected to absorb
          </p>
          <div className="flex gap-2">
            <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button
              className="gap-2"
              disabled={busy || selected.size === 0}
              onClick={() => void confirm()}
            >
              <GitMerge className="size-3.5" />
              {busy ? "Merging…" : "Merge"}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Thumb({
  projectId,
  person,
  cacheRev,
}: {
  projectId: string
  person: Person
  cacheRev: string
}) {
  const src = person.thumbnail_path
    ? appendQuery(api.mediaUrl(projectId, person.thumbnail_path), {
        v: `${cacheRev}-${person.detection_count}`,
      })
    : null
  return <FacePortrait src={src} size="sm" fit="cover" />
}
