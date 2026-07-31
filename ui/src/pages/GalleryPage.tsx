import { useMemo, useState } from "react"
import { useNavigate } from "react-router-dom"
import { Film, GitMerge, ImagePlus, Loader2, ScanFace, X } from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { PageHeader } from "@/components/app/PageHeader"
import { EmptyState } from "@/components/app/EmptyState"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { CastPackDialog } from "@/components/gallery/CastPackDialog"
import { CastTile } from "@/components/gallery/CastTile"
import { MergeCastDialog } from "@/components/gallery/MergeCastDialog"
import { PersonDossier } from "@/components/gallery/PersonDossier"
import { api } from "@/lib/api"
import { buildSharedScenesClient } from "@/lib/scenes"
import { sanitizeStage } from "@/lib/plainCopy"
import { useProjects } from "@/state/project"

export function GalleryPage() {
  const nav = useNavigate()
  const {
    active,
    activeId,
    projectsReady,
    hydrating,
    setSelectedPersonId,
    selectedPersonId,
    refreshActive,
  } = useProjects()

  const selected = useMemo(
    () => active?.people.find((p) => p.person_id === selectedPersonId) ?? null,
    [active, selectedPersonId],
  )

  const liveScan =
    active?.status === "cutting" ||
    active?.status === "scanning" ||
    active?.status === "paused"
  const ready = active?.status === "ready"
  const people = active?.people ?? []
  const scenes = useMemo(
    () =>
      active?.scenes?.length
        ? active.scenes
        : buildSharedScenesClient(active?.segments ?? []),
    [active?.scenes, active?.segments],
  )
  const cacheRev = `${active?.detect_hits ?? 0}-${active?.updated_at ?? ""}`

  const [selectMode, setSelectMode] = useState(false)
  const [picked, setPicked] = useState<Set<number>>(new Set())
  const [mergeOpen, setMergeOpen] = useState(false)
  const [mergeKeepId, setMergeKeepId] = useState<number | null>(null)
  const [mergeInitial, setMergeInitial] = useState<number[]>([])
  const [packOpen, setPackOpen] = useState(false)

  const openMerge = (keepId: number, initial: number[] = []) => {
    setMergeKeepId(keepId)
    setMergeInitial(initial)
    setMergeOpen(true)
  }

  const exitSelectMode = () => {
    setSelectMode(false)
    setPicked(new Set())
  }

  const togglePick = (id: number) => {
    setPicked((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const mergeFromSelection = () => {
    if (picked.size < 2) {
      toast.message("Pick at least two characters to merge")
      return
    }
    const ids = [...picked]
    const keep =
      ids
        .map((id) => people.find((p) => p.person_id === id))
        .filter(Boolean)
        .sort((a, b) => (b!.detection_count || 0) - (a!.detection_count || 0))[0]
        ?.person_id ?? ids[0]
    openMerge(
      keep,
      ids.filter((id) => id !== keep),
    )
  }

  if (!active) {
    if (!projectsReady || hydrating || activeId) {
      return (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-8">
          <div className="w-full max-w-sm">
            <LoadingBlock label="Loading project…" rows={4} dense />
          </div>
        </div>
      )
    }
    return (
      <EmptyState
        title="No project selected"
        description="Import footage or pick a project from the dashboard."
        actionLabel="Go to import"
        onAction={() => nav("/import")}
        icon={<Film className="size-6" />}
      />
    )
  }

  if (liveScan && people.length === 0) {
    return (
      <EmptyState
        title="Building cast…"
        description={
          sanitizeStage(active.stage) ||
          "Characters appear as faces are found."
        }
        actionLabel="View import"
        onAction={() => nav("/import")}
        icon={<Loader2 className="size-6 animate-spin" />}
      />
    )
  }

  if (!liveScan && active.status === "error") {
    return (
      <EmptyState
        title="Scan failed"
        description="Open Import to retry, or check Settings → Packs."
        actionLabel="Open Import"
        onAction={() => nav("/import")}
        icon={<ScanFace className="size-6" />}
      />
    )
  }

  if (!liveScan && active.status === "idle") {
    return (
      <EmptyState
        title="Scan not started"
        description="Open Import to start finding characters in this video."
        actionLabel="Open Import"
        onAction={() => nav("/import")}
        icon={<ScanFace className="size-6" />}
      />
    )
  }

  if (!liveScan && (active.status !== "ready" || !people.length)) {
    return (
      <EmptyState
        title="No characters yet"
        description="Import and scan a video to populate the gallery."
        actionLabel="Import video"
        onAction={() => nav("/import")}
        icon={<ScanFace className="size-6" />}
      />
    )
  }

  return (
    <div className="cast-bay flex min-h-0 flex-1 flex-col overflow-hidden">
      <PageHeader
        eyebrow={active.name}
        title="Cast"
        description={
          liveScan
            ? "Live roster while the scan runs. Characters sharpen as more faces are found."
            : "Talent wall. Name characters, teach faces, merge doubles, play scene reels."
        }
        actions={
          <div className="flex flex-wrap items-center gap-2">
            {liveScan ? (
              <Badge
                variant="secondary"
                className={
                  active.status === "paused"
                    ? "gap-1.5 border-amber-500/30 text-amber-400"
                    : "gap-1.5 border-primary/25 text-primary"
                }
              >
                {active.status === "paused" ? null : (
                  <Loader2 className="size-3 animate-spin" />
                )}
                {active.status === "paused" ? "Paused" : "Live"} ·{" "}
                {Math.round((active.progress || 0) * 100)}%
              </Badge>
            ) : null}
            <Badge variant="secondary" className="timecode tabular-nums">
              {people.length} cast
            </Badge>
            {ready && people.length > 0 ? (
              <Button
                size="sm"
                variant="outline"
                className="gap-1.5"
                onClick={() => setPackOpen(true)}
              >
                <ImagePlus className="size-3.5" />
                Cast photos
              </Button>
            ) : null}
            {ready && people.length > 1 ? (
              selectMode ? (
                <>
                  <Button size="sm" variant="outline" className="gap-1.5" onClick={exitSelectMode}>
                    <X className="size-3.5" />
                    Cancel
                  </Button>
                  <Button
                    size="sm"
                    className="gap-1.5"
                    disabled={picked.size < 2}
                    onClick={mergeFromSelection}
                  >
                    <GitMerge className="size-3.5" />
                    Merge ({picked.size})
                  </Button>
                </>
              ) : (
                <Button
                  size="sm"
                  variant="outline"
                  className="gap-1.5"
                  onClick={() => setSelectMode(true)}
                >
                  <GitMerge className="size-3.5" />
                  Multi-merge
                </Button>
              )
            ) : null}
          </div>
        }
      />

      {liveScan ? (
        <div className="border-b border-primary/15 bg-primary/5 px-5 py-1.5 text-[11px] text-muted-foreground sm:px-6">
          <span className="text-primary">{sanitizeStage(active.stage) || "Scanning"}</span>
          {" · "}
          {active.detect_hits ?? 0} faces · export after scan finishes
        </div>
      ) : null}

      {ready && active.needs_rescan ? (
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-amber-500/25 bg-amber-500/10 px-5 py-2 text-[11px] text-amber-100/90 sm:px-6">
          <span>
            Cast from older identity pipeline. Re-scan for stricter face matching
            (wrong merges stay until then).
          </span>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 border-amber-500/40 text-amber-100"
            onClick={() => {
              void api
                .scan(active.id)
                .then(() => refreshActive())
                .catch((err: unknown) =>
                  toast.error(err instanceof Error ? err.message : "Scan failed"),
                )
            }}
          >
            <ScanFace className="size-3.5" />
            Re-scan
          </Button>
        </div>
      ) : null}

      {selectMode ? (
        <div className="border-b border-primary/20 bg-primary/5 px-5 py-1.5 text-[11px] text-muted-foreground sm:px-6">
          Multi-select. Survivor = most appearances. Scenes stay in timeline order.
        </div>
      ) : null}

      <div className="grid min-h-0 flex-1 grid-cols-1 overflow-hidden lg:grid-cols-[minmax(0,1fr)_minmax(0,min(380px,38vw))]">
        <section className="flex min-h-0 min-w-0 flex-col overflow-hidden border-b border-border/40 lg:border-b-0 lg:border-r">
          <div className="flex items-center justify-between gap-3 border-b border-border/35 px-5 py-2.5 sm:px-6">
            <div>
              <p className="text-[10px] uppercase tracking-[0.2em] text-muted-foreground">
                Talent wall
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground/80">
                Dense roster · select a face for the character sheet
              </p>
            </div>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            <div className="grid grid-cols-2 gap-2 p-4 sm:grid-cols-3 md:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 sm:p-5">
              {people.map((person, i) => (
                <CastTile
                  key={person.person_id}
                  projectId={active.id}
                  person={person}
                  scenes={scenes}
                  cacheRev={cacheRev}
                  index={i}
                  selected={person.person_id === selectedPersonId}
                  selectMode={selectMode}
                  picked={picked.has(person.person_id)}
                  onClick={() => {
                    if (selectMode) {
                      togglePick(person.person_id)
                      return
                    }
                    setSelectedPersonId(person.person_id)
                  }}
                />
              ))}
            </div>
          </ScrollArea>
        </section>

        <aside className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-card/20">
          {selected ? (
            <ScrollArea className="min-h-0 min-w-0 flex-1">
              <PersonDossier
                projectId={active.id}
                people={people}
                person={selected}
                segments={active.segments}
                onSelect={setSelectedPersonId}
                onDeselect={() => setSelectedPersonId(null)}
                onExport={() => {
                  if (liveScan) return
                  setSelectedPersonId(selected.person_id)
                  nav("/export")
                }}
                onTimeline={() => {
                  if (liveScan) return
                  setSelectedPersonId(selected.person_id)
                  nav("/timeline")
                }}
                onRename={
                  liveScan
                    ? undefined
                    : async (label) => {
                        await api.renamePerson(active.id, selected.person_id, label)
                        await refreshActive()
                      }
                }
                onMerge={() => openMerge(selected.person_id)}
                onTaught={async () => {
                  await refreshActive()
                }}
                showTimelineCta={!liveScan}
                showExportCta={!liveScan}
                showMergeCta={ready && people.length > 1}
                showReel={ready}
                showTeach={ready}
              />
            </ScrollArea>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
              <div className="flex size-14 items-center justify-center rounded-full border border-border/40 bg-background/40">
                <ScanFace className="size-6 text-muted-foreground/45" />
              </div>
              <div className="space-y-1">
                <p className="font-heading text-base tracking-tight">Character sheet</p>
                <p className="max-w-[22ch] text-xs leading-relaxed text-muted-foreground">
                  {liveScan
                    ? "Pick a face from the wall. Roster updates while scanning."
                    : selectMode
                      ? "Tap faces to pick, then merge."
                      : "Pick a face to open reel, teach, and export."}
                </p>
              </div>
            </div>
          )}
        </aside>
      </div>

      {mergeKeepId != null ? (
        <MergeCastDialog
          open={mergeOpen}
          onOpenChange={(open) => {
            setMergeOpen(open)
            if (!open) {
              setMergeKeepId(null)
              setMergeInitial([])
            }
          }}
          projectId={active.id}
          people={people}
          keepPersonId={mergeKeepId}
          initialMergeIds={mergeInitial}
          cacheRev={cacheRev}
          onMerged={async (keepId) => {
            await refreshActive()
            setSelectedPersonId(keepId)
            exitSelectMode()
          }}
        />
      ) : null}

      <CastPackDialog
        open={packOpen}
        onOpenChange={setPackOpen}
        projectId={active.id}
        onDone={async () => {
          await refreshActive()
        }}
      />
    </div>
  )
}
