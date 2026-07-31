import { useMemo, useState } from "react"
import { useNavigate } from "react-router-dom"
import { Clapperboard } from "lucide-react"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar"
import { PageHeader } from "@/components/app/PageHeader"
import { EmptyState } from "@/components/app/EmptyState"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { PersonDossier } from "@/components/gallery/PersonDossier"
import { SegmentTimeline } from "@/components/timeline/SegmentTimeline"
import { VideoPreview } from "@/components/timeline/VideoPreview"
import { CopyButton } from "@/components/shared/CopyButton"
import { api, formatDuration } from "@/lib/api"
import { formatSegmentsForCopy } from "@/lib/clipboard"
import { useProjects } from "@/state/project"
import { cn } from "@/lib/utils"

export function TimelinePage() {
  const nav = useNavigate()
  const {
    active,
    activeId,
    projectsReady,
    selectedPersonId,
    setSelectedPersonId,
    refreshActive,
  } = useProjects()
  const [activeSegment, setActiveSegment] = useState<number | null>(0)

  const person =
    active?.people.find((p) => p.person_id === selectedPersonId) ??
    active?.people[0] ??
    null

  const segments = useMemo(() => {
    if (!active || !person) return []
    return active.segments
      .filter((s) => s.person_id === person.person_id)
      .sort((a, b) => a.start_sec - b.start_sec)
  }, [active, person])

  const seekTime =
    activeSegment !== null && segments[activeSegment]
      ? segments[activeSegment].start_sec
      : undefined

  if (!active || active.status !== "ready") {
    if (!projectsReady || (Boolean(activeId) && !active)) {
      return (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-8">
          <div className="w-full max-w-sm">
            <LoadingBlock label="Loading timeline…" rows={4} dense />
          </div>
        </div>
      )
    }
    return (
      <EmptyState
        title="Timeline unavailable"
        description="Finish scanning a project before reviewing segments."
        actionLabel="Import footage"
        onAction={() => nav("/import")}
      />
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <PageHeader
        eyebrow={active.name}
        title="Timeline"
        description="Scrub source video, click segments to seek, copy timecodes for edit suites."
        actions={
          person ? (
            <div className="flex gap-2">
              <CopyButton
                text={formatSegmentsForCopy(segments, person.label)}
                label="Copy timecodes"
                toastLabel="Segment list copied"
              />
              <Button size="sm" className="gap-1.5" onClick={() => nav("/export")}>
                <Clapperboard className="size-3.5" />
                Export
              </Button>
            </div>
          ) : null
        }
      />

      <div className="grid min-h-0 flex-1 lg:grid-cols-[200px_minmax(0,1fr)_320px]">
        <aside className="hidden min-h-0 border-r border-border/60 lg:flex lg:flex-col">
          <p className="px-4 py-3 text-[10px] uppercase tracking-wider text-muted-foreground">
            Cast
          </p>
          <ScrollArea className="min-h-0 flex-1 px-2 pb-4">
            <div className="space-y-1">
              {active.people.map((p) => (
                <button
                  key={p.person_id}
                  type="button"
                  onClick={() => {
                    setSelectedPersonId(p.person_id)
                    setActiveSegment(0)
                  }}
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition",
                    person?.person_id === p.person_id
                      ? "bg-sidebar-accent text-foreground"
                      : "text-muted-foreground hover:bg-muted/40",
                  )}
                >
                  <Avatar className="size-8 rounded-md">
                    {p.thumbnail_path ? (
                      <AvatarImage src={api.mediaUrl(active.id, p.thumbnail_path)} alt={p.label} />
                    ) : null}
                    <AvatarFallback className="rounded-md text-[10px]">
                      {p.label.slice(0, 2).toUpperCase()}
                    </AvatarFallback>
                  </Avatar>
                  <span className="truncate">{p.label}</span>
                </button>
              ))}
            </div>
          </ScrollArea>
        </aside>

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden border-r border-border/40">
          {person ? (
            <div className="grid min-h-0 flex-1 gap-4 p-4 lg:grid-rows-[minmax(0,1.1fr)_auto] lg:p-6">
              <VideoPreview src={api.videoUrl(active.id)} currentTime={seekTime} className="h-full" />

              <div className="flex min-h-0 flex-col gap-3">
                <SegmentTimeline
                  segments={segments}
                  durationSec={active.duration_sec}
                  activeIndex={activeSegment}
                  onSelect={(i) => setActiveSegment(i)}
                />

                <ScrollArea className="max-h-48 min-h-0">
                  <div className="space-y-1.5 pr-2">
                    {segments.map((s, i) => (
                      <button
                        key={`${s.start_sec}-${i}`}
                        type="button"
                        onClick={() => setActiveSegment(i)}
                        className={cn(
                          "flex w-full items-center justify-between rounded-lg border px-3 py-2.5 text-left transition",
                          activeSegment === i
                            ? "border-primary/40 bg-primary/5"
                            : "border-border/40 bg-card/30 hover:bg-card/50",
                        )}
                      >
                        <span className="text-sm font-medium">
                          Clip {String(i + 1).padStart(2, "0")}
                        </span>
                        <span className="timecode text-xs text-muted-foreground">
                          {formatDuration(s.start_sec)} → {formatDuration(s.end_sec)} ·{" "}
                          {formatDuration(s.end_sec - s.start_sec)}
                        </span>
                      </button>
                    ))}
                  </div>
                </ScrollArea>
              </div>
            </div>
          ) : (
            <p className="p-8 text-muted-foreground">No person selected.</p>
          )}
        </div>

        <aside className="hidden min-h-0 border-l border-border/60 bg-card/25 xl:flex xl:flex-col">
          {person ? (
            <ScrollArea className="min-h-0 flex-1">
              <PersonDossier
                projectId={active.id}
                people={active.people}
                person={person}
                onSelect={(id) => {
                  setSelectedPersonId(id)
                  setActiveSegment(0)
                }}
                allowDeselect={false}
                showTimelineCta={false}
                onExport={() => {
                  setSelectedPersonId(person.person_id)
                  nav("/export")
                }}
                onRename={async (label) => {
                  await api.renamePerson(active.id, person.person_id, label)
                  await refreshActive()
                }}
              />
            </ScrollArea>
          ) : null}
        </aside>
      </div>
    </div>
  )
}
