import { useMemo, useState } from "react"
import { AnimatePresence, motion } from "motion/react"
import {
  ChevronLeft,
  ChevronRight,
  Clapperboard,
  GitMerge,
  ListVideo,
  Pencil,
} from "lucide-react"
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { SceneReelPlayer } from "@/components/gallery/SceneReelPlayer"
import { TeachFacePanel } from "@/components/gallery/TeachFacePanel"
import { FacePortrait } from "@/components/shared/FacePortrait"
import { api, formatDuration, type Person, type Segment } from "@/lib/api"
import { cn } from "@/lib/utils"

export function qualityForDetections(count: number): { label: string; tone: string } {
  if (count >= 50)
    return { label: "Excellent", tone: "border-emerald-500/30 bg-emerald-500/10 text-emerald-400" }
  if (count >= 20)
    return { label: "Strong", tone: "border-primary/30 bg-primary/10 text-primary" }
  if (count >= 8)
    return { label: "Fair", tone: "border-amber-500/30 bg-amber-500/10 text-amber-400" }
  return { label: "Sparse", tone: "border-border/50 bg-muted/40 text-muted-foreground" }
}

type PersonDossierProps = {
  projectId: string
  people: Person[]
  person: Person
  segments?: Segment[]
  onSelect: (personId: number) => void
  onDeselect?: () => void
  onExport: () => void
  onTimeline?: () => void
  onRename?: (label: string) => Promise<void>
  onMerge?: () => void
  onTaught?: () => Promise<void> | void
  allowDeselect?: boolean
  showTimelineCta?: boolean
  showExportCta?: boolean
  showMergeCta?: boolean
  showReel?: boolean
  showTeach?: boolean
  className?: string
}

export function PersonDossier({
  projectId,
  people,
  person,
  segments = [],
  onSelect,
  onDeselect,
  onExport,
  onTimeline,
  onRename,
  onMerge,
  onTaught,
  allowDeselect = true,
  showTimelineCta = true,
  showExportCta = true,
  showMergeCta = false,
  showReel = false,
  showTeach = false,
  className,
}: PersonDossierProps) {
  const [renaming, setRenaming] = useState(false)
  const [draft, setDraft] = useState(person.label)
  const [busy, setBusy] = useState(false)

  const index = Math.max(
    0,
    people.findIndex((p) => p.person_id === person.person_id),
  )
  const quality = useMemo(
    () => qualityForDetections(person.detection_count),
    [person.detection_count],
  )

  const personSegments = useMemo(
    () => segments.filter((s) => s.person_id === person.person_id),
    [segments, person.person_id],
  )

  const go = (delta: number) => {
    if (!people.length) return
    const next = (index + delta + people.length) % people.length
    onSelect(people[next].person_id)
    setRenaming(false)
    setDraft(people[next].label)
  }

  return (
    <div className={cn("flex h-full min-h-0 min-w-0 max-w-full flex-col overflow-x-hidden", className)}>
      <AnimatePresence mode="wait">
        <motion.div
          key={person.person_id}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 }}
          transition={{ type: "spring", stiffness: 340, damping: 30 }}
          className="flex min-h-0 min-w-0 max-w-full flex-1 flex-col gap-4 overflow-x-hidden p-4 sm:p-5"
        >
          {/* Identity strip — face + name, not a giant hero */}
          <div className="flex min-w-0 items-start gap-3">
            <FacePortrait
              src={
                person.thumbnail_path
                  ? api.mediaUrl(projectId, person.thumbnail_path)
                  : null
              }
              alt={person.label}
              size="md"
              fit="contain"
              className="shrink-0"
            />
            <div className="min-w-0 flex-1 space-y-2 pt-0.5">
              <div className="flex items-center justify-between gap-1">
                <p className="timecode text-[10px] tabular-nums text-muted-foreground">
                  {String(index + 1).padStart(2, "0")} /{" "}
                  {String(people.length).padStart(2, "0")}
                </p>
                {people.length > 1 ? (
                  <div className="flex items-center gap-0.5">
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="size-7"
                      onClick={() => go(-1)}
                      aria-label="Previous character"
                    >
                      <ChevronLeft className="size-3.5" />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="size-7"
                      onClick={() => go(1)}
                      aria-label="Next character"
                    >
                      <ChevronRight className="size-3.5" />
                    </Button>
                  </div>
                ) : null}
              </div>

              {renaming && onRename ? (
                <div className="flex gap-1.5">
                  <Input
                    value={draft}
                    autoFocus
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        setRenaming(false)
                        setDraft(person.label)
                      }
                    }}
                    className="h-8 bg-background/50"
                  />
                  <Button
                    size="sm"
                    className="h-8"
                    disabled={!draft.trim() || busy}
                    onClick={async () => {
                      if (!draft.trim() || !onRename) return
                      setBusy(true)
                      try {
                        await onRename(draft.trim())
                        setRenaming(false)
                      } finally {
                        setBusy(false)
                      }
                    }}
                  >
                    Save
                  </Button>
                </div>
              ) : (
                <div className="flex items-start gap-1">
                  <h2 className="font-heading min-w-0 flex-1 text-xl leading-tight tracking-tight">
                    {person.label}
                  </h2>
                  {onRename ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="mt-0.5 size-7 shrink-0 text-muted-foreground"
                      onClick={() => {
                        setDraft(person.label)
                        setRenaming(true)
                      }}
                      aria-label="Rename character"
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                  ) : null}
                </div>
              )}

              <span
                className={cn(
                  "inline-flex rounded-md border px-2 py-0.5 text-[10px] font-medium",
                  quality.tone,
                )}
              >
                {quality.label} · {person.detection_count} faces
              </span>
            </div>
          </div>

          {/* Compact stats row — dividers, not card stack */}
          <dl className="grid grid-cols-4 gap-px overflow-hidden rounded-lg border border-border/40 bg-border/40">
            <StatCell value={String(person.clip_count)} label="Scenes" />
            <StatCell value={formatDuration(person.screen_time_sec)} label="Time" />
            <StatCell value={formatDuration(person.first_sec)} label="In" />
            <StatCell value={formatDuration(person.last_sec)} label="Out" />
          </dl>

          {showReel ? (
            <div className="min-w-0 space-y-2 overflow-hidden border-t border-border/35 pt-3">
              <p className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                Scene reel
              </p>
              <SceneReelPlayer
                projectId={projectId}
                segments={personSegments}
                allSegments={segments}
                people={people}
                onSelectPerson={onSelect}
                className="min-w-0 max-w-full"
              />
            </div>
          ) : null}

          {showTeach && onTaught ? (
            <TeachFacePanel
              key={person.person_id}
              projectId={projectId}
              personId={person.person_id}
              personLabel={person.label}
              onTaught={onTaught}
              className="min-w-0 max-w-full"
            />
          ) : null}

          <div className="mt-auto flex min-w-0 flex-col gap-1.5 overflow-hidden border-t border-border/35 pt-3">
            {showExportCta ? (
              <AnimatedButton
                className="h-10 w-full max-w-full gap-2"
                hoverScale={1}
                tapScale={0.98}
                onClick={onExport}
              >
                <Clapperboard className="size-4" />
                Export ScenePack
              </AnimatedButton>
            ) : (
              <p className="rounded-md border border-border/35 bg-background/30 px-3 py-2 text-center text-xs text-muted-foreground">
                Export unlocks when the scan finishes.
              </p>
            )}
            <div className="grid grid-cols-2 gap-1.5">
              {showMergeCta && onMerge && people.length > 1 ? (
                <Button variant="outline" className="h-9 gap-1.5" onClick={onMerge}>
                  <GitMerge className="size-3.5" />
                  Merge
                </Button>
              ) : (
                <span />
              )}
              {showTimelineCta && onTimeline ? (
                <Button variant="outline" className="h-9 gap-1.5" onClick={onTimeline}>
                  <ListVideo className="size-3.5" />
                  Timeline
                </Button>
              ) : null}
            </div>
            {allowDeselect && onDeselect ? (
              <button
                type="button"
                onClick={onDeselect}
                className="py-1.5 text-center text-[11px] text-muted-foreground transition hover:text-foreground"
              >
                Deselect
              </button>
            ) : null}
          </div>
        </motion.div>
      </AnimatePresence>
    </div>
  )
}

function StatCell({ value, label }: { value: string; label: string }) {
  return (
    <div className="bg-card/80 px-2 py-2.5 text-center">
      <dd className="timecode text-sm font-medium tabular-nums text-primary">{value}</dd>
      <dt className="mt-0.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground">
        {label}
      </dt>
    </div>
  )
}
