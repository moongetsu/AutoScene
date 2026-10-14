import { useState } from "react"
import { Link, useParams } from "@tanstack/react-router"
import { ChevronDown, ScanFace, Sparkles, UserRound, Users } from "lucide-react"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { Page, PageHeader } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { formatPercent } from "@/lib/format"
import { assetUrl } from "@/lib/media"
import { useCast, useMediaList, useProject } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { formatScreenTime, plural, splitHidden, useProjectAnalysis } from "./cast"

/** Everyone the analysis found, most screen time first. */
export function CastPage() {
  const { pid } = useParams({ from: "/p/$pid/cast" })
  const { data: project } = useProject(pid)
  const { data: cast, isPending } = useCast(pid)
  const { data: media } = useMediaList(pid)
  const running = useProjectAnalysis(pid)
  const [showHidden, setShowHidden] = useState(false)
  const { shown, hidden } = splitHidden(cast)
  const anime = project?.content === "anime"

  const description = cast && (
    <>
      {plural(shown.length, "character")}
      {cast.mediaTotal > 0 && (
        <>
          {" "}
          · faces from {cast.mediaDone} of {plural(cast.mediaTotal, project?.kind === "movie" ? "file" : "episode")}
        </>
      )}
    </>
  )

  return (
    <Page>
      <PageHeader
        title="Cast"
        description={description}
        actions={
          cast && cast.unsure > 0 ? (
            <Button variant="primary" asChild>
              <Link to="/p/$pid/cast/review" params={{ pid }}>
                <Sparkles /> Review {plural(cast.unsure, "face")}
              </Link>
            </Button>
          ) : null
        }
      />
      <div className="px-8 pb-10">
        <AnalysisBanner pid={pid} jobs={running} />
        {anime ? (
          <EmptyState
            icon={<ScanFace />}
            title="Anime characters are coming"
            description="Characters are found in live action for now. Anime gets its own face and head detectors in an upcoming update; shots already work."
            className="rounded-xl border border-hairline bg-panel"
          />
        ) : isPending ? (
          <CardGrid>
            {Array.from({ length: 8 }, (_, i) => (
              <div key={i} className="skeleton aspect-[4/5] rounded-lg" />
            ))}
          </CardGrid>
        ) : shown.length === 0 ? (
          <EmptyState
            icon={<Users />}
            title={running.length > 0 ? "Looking for characters…" : "No characters yet"}
            description={
              (media?.length ?? 0) === 0
                ? "Add episodes to this project. Characters appear here as they are analysed."
                : "Characters appear here as soon as the first episode's faces are analysed."
            }
            className="rounded-xl border border-hairline bg-panel"
          />
        ) : (
          <>
            <CardGrid>
              {shown.map((c) => (
                <CharacterCard key={c.id} pid={pid} character={c} />
              ))}
              {cast && cast.unassigned > 0 && (
                <Link
                  to="/p/$pid/cast/$cid"
                  params={{ pid, cid: "unassigned" }}
                  className="group flex aspect-[4/5] flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-soft text-center transition-colors hover:border-strong hover:bg-hover"
                >
                  <UserRound className="size-6 text-fg-3 group-hover:text-fg-2" />
                  <span className="text-sm text-fg-2">Everyone else</span>
                  <span className="text-2xs text-fg-3">{plural(cast.unassigned, "face")}</span>
                </Link>
              )}
            </CardGrid>
            {hidden.length > 0 && (
              <div className="mt-8">
                <button
                  onClick={() => setShowHidden((v) => !v)}
                  className="eyebrow flex items-center gap-1.5 hover:text-fg-1"
                  aria-expanded={showHidden}
                >
                  <ChevronDown className={cn("size-3.5 transition-transform", !showHidden && "-rotate-90")} />
                  Hidden · {hidden.length}
                </button>
                {showHidden && (
                  <CardGrid className="mt-3 opacity-70">
                    {hidden.map((c) => (
                      <CharacterCard key={c.id} pid={pid} character={c} />
                    ))}
                  </CardGrid>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </Page>
  )
}

function CardGrid({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-4", className)}>{children}</div>
}

function CharacterCard({ pid, character: c }: { pid: string; character: Character }) {
  const [failed, setFailed] = useState(false)
  // Their episode is being analysed again: the faces come back when it's done.
  const refinding = c.tracks === 0 && c.waiting > 0
  return (
    <Link
      to="/p/$pid/cast/$cid"
      params={{ pid, cid: c.id }}
      className={cn(
        "group flex flex-col overflow-hidden rounded-lg border border-hairline bg-panel shadow-1 transition-[border-color,transform,opacity] duration-[var(--dur-fast)] hover:-translate-y-px hover:border-strong focus-visible:border-accent-line focus-visible:outline-none",
        refinding && "opacity-60",
      )}
    >
      <div className="relative aspect-square bg-inset">
        {c.cover && !failed ? (
          <img
            src={assetUrl(c.cover)}
            alt=""
            draggable={false}
            onError={() => setFailed(true)}
            className="size-full object-cover"
          />
        ) : refinding ? (
          <ScanFace className="absolute inset-0 m-auto size-10 text-fg-3" />
        ) : (
          <UserRound className="absolute inset-0 m-auto size-10 text-fg-3" />
        )}
        {c.unsure > 0 && (
          <span className="absolute top-2 right-2 rounded-full bg-canvas/80 px-1.5 text-[10px] leading-5 text-fg-2">
            +{c.unsure} to review
          </span>
        )}
      </div>
      <div className="px-3 py-2.5">
        <div className={cn("truncate text-sm font-medium", c.name ? "text-fg-1" : "text-fg-2")}>{c.label}</div>
        <div className="mt-0.5 truncate text-2xs text-fg-3">
          {refinding ? (
            "Being found again…"
          ) : (
            <>
              {formatScreenTime(c.screenSec ?? 0)} · {plural(c.shots, "shot")}
              {c.media > 1 && <> · {c.media} eps</>}
            </>
          )}
        </div>
      </div>
    </Link>
  )
}

/** Which episodes are being analysed right now. */
function AnalysisBanner({ pid, jobs }: { pid: string; jobs: ReturnType<typeof useProjectAnalysis> }) {
  const { data: media } = useMediaList(pid)
  const job = jobs.find((j) => j.state === "running") ?? jobs[0]
  if (!job) return null
  const m = media?.find((x) => x.id === job.mediaId)
  const waiting = jobs.length - 1
  return (
    <div className="mb-6 flex items-center gap-4 rounded-lg border border-hairline bg-panel px-4 py-3 shadow-1">
      <ScanFace className="size-4 shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-3 text-xs">
          <span className="truncate text-fg-1">
            {job.state === "queued" ? "Waiting to analyse" : (job.stage ?? "Analysing")}
            {m && <span className="text-fg-3"> · {m.label || m.fileName}</span>}
          </span>
          <span className="timecode shrink-0 text-fg-3">
            {formatPercent(job.progress)}
            {waiting > 0 && <> · {plural(waiting, "more episode")} queued</>}
          </span>
        </div>
        <Progress className="mt-1.5" value={job.state === "running" ? job.progress : null} />
      </div>
    </div>
  )
}
