import type * as React from "react"
import { useState } from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import { CaretDown, Eye, EyeSlash, Info, MagnifyingGlass, User, UserFocus, Users } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Page } from "@/components/ui/page"
import { TopBar } from "@/components/ui/topbar"
import { AnalysisStatus } from "@/features/project/AnalysisStatus"
import { ProjectLoadError } from "@/features/project/ProjectLoadError"
import { ProjectNav } from "@/features/project/ProjectNav"
import { firstFailure, useCast, useMediaList, useProject, useSettings } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { CharacterCard } from "./CharacterCard"
import { AniListDialog } from "./AniListDialog"
import { AniListNames } from "./AniListNames"
import { namesSource, plural, splitHidden, unplacedNote, useProjectAnalysis } from "./cast"

/** Everyone the analysis found, most screen time first. */
export function CastPage() {
  const { pid } = useParams({ from: "/p/$pid/cast" })
  const projectQuery = useProject(pid)
  const castQuery = useCast(pid)
  const mediaQuery = useMediaList(pid)
  const { data: project } = projectQuery
  const { data: cast, isPending } = castQuery
  const { data: media } = mediaQuery
  const { data: settings } = useSettings()
  const running = useProjectAnalysis(pid)
  const [showHidden, setShowHidden] = useState(false)
  const [anilistOpen, setAnilistOpen] = useState(false)
  const navigate = useNavigate()
  const { shown, hidden } = splitHidden(cast)
  const unplaced = unplacedNote(cast?.unplaced ?? 0)
  // Names from AniList (anime) or TVmaze (animation) while the opt-in in
  // Settings is on and someone shown is still unnamed.
  const source = namesSource(project?.content, project?.kind)
  const anilist = !!settings?.anilist && source !== null && !!cast?.characters.some((c) => !c.hidden && !c.name)
  const failure = firstFailure(projectQuery, castQuery, mediaQuery)

  if (failure) {
    return (
      <Page topbar={<TopBar title={project?.title} nav={<ProjectNav pid={pid} />} />}>
        <ProjectLoadError error={failure.error} onRetry={failure.retry} />
      </Page>
    )
  }

  return (
    <Page
      topbar={<TopBar title={project?.title} nav={<ProjectNav pid={pid} />} />}
      menu={
        <>
          {cast && cast.unsure > 0 && (
            <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid/cast/review", params: { pid } })}>
              <UserFocus /> Review {plural(cast.unsure, "face")}
            </ContextMenuItem>
          )}
          {cast && cast.unassigned > 0 && (
            <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: "unassigned" } })}>
              <User /> Everyone else
            </ContextMenuItem>
          )}
          {anilist && (
            <ContextMenuItem onSelect={() => setAnilistOpen(true)}>
              <MagnifyingGlass /> Names from {source}
            </ContextMenuItem>
          )}
          {hidden.length > 0 && (
            <ContextMenuItem onSelect={() => setShowHidden((v) => !v)}>
              {showHidden ? <EyeSlash /> : <Eye />} {showHidden ? "Fold the hidden characters" : "Show the hidden characters"}
            </ContextMenuItem>
          )}
        </>
      }
    >
      <header className="flex flex-wrap items-end justify-between gap-6 px-10 pt-10 pb-8">
        <div>
          <h1 className="display text-5xl text-fg-1">Cast</h1>
          {cast && (
            <p className="mt-2 text-md text-fg-2">
              {plural(shown.length, "character")}
              {cast.mediaTotal > 0 &&
                ` · faces from ${cast.mediaDone} of ${plural(cast.mediaTotal, project?.kind === "movie" ? "file" : "episode")}`}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2.5">
          {anilist && (
            <Button variant="secondary" size="lg" onClick={() => setAnilistOpen(true)}>
              <MagnifyingGlass weight="bold" /> Names from {source}
            </Button>
          )}
          {cast && cast.unsure > 0 && (
            <Button variant="primary" size="lg" asChild>
              <Link to="/p/$pid/cast/review" params={{ pid }}>
                <UserFocus weight="bold" /> Review {plural(cast.unsure, "face")}
              </Link>
            </Button>
          )}
        </div>
      </header>

      <div className="px-10 pb-16">
        <AnalysisStatus pid={pid} className="mb-8" />
        <AniListNames pid={pid} cast={cast} className="mb-8" />
        {unplaced && (
          <p className="mb-8 flex max-w-2xl items-start gap-2.5 rounded-lg border border-white/10 bg-white/[0.04] px-3.5 py-3 text-sm text-fg-2 [&_svg]:mt-0.5 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-1">
            <Info /> {unplaced}
          </p>
        )}
        {isPending ? (
          <CardGrid>
            {Array.from({ length: 8 }, (_, i) => (
              <div key={i} className="skeleton aspect-[3/4] rounded-2xl" />
            ))}
          </CardGrid>
        ) : shown.length === 0 ? (
          <EmptyState
            icon={<Users />}
            title={running.length > 0 ? "Looking for characters" : "No characters yet"}
            description={
              (media?.length ?? 0) === 0
                ? "Add episodes to this project. Characters appear here as they are analysed."
                : "Characters appear here as soon as the first episode's faces are analysed."
            }
          />
        ) : (
          <>
            <CardGrid>
              {shown.map((c) => (
                <CharacterCard key={c.id} pid={pid} character={c} />
              ))}
              {cast && cast.unassigned > 0 && (
                <ContextArea
                  menu={
                    <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: "unassigned" } })}>
                      <User /> Open everyone else
                    </ContextMenuItem>
                  }
                >
                  <Link
                    to="/p/$pid/cast/$cid"
                    params={{ pid, cid: "unassigned" }}
                    className="group flex aspect-[3/4] flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-white/15 text-center transition-colors hover:border-white/30 hover:bg-white/[0.03]"
                  >
                    <span className="flex size-14 items-center justify-center rounded-full bg-white/[0.06] text-fg-2 transition-colors group-hover:bg-white/[0.1] group-hover:text-fg-1 [&_svg]:size-6">
                      <User />
                    </span>
                    <span className="display text-2xl text-fg-1">Everyone else</span>
                    <span className="text-xs text-fg-3">{plural(cast.unassigned, "face")} nobody claimed</span>
                  </Link>
                </ContextArea>
              )}
            </CardGrid>
            {hidden.length > 0 && (
              <div className="mt-12">
                <button
                  onClick={() => setShowHidden((v) => !v)}
                  className="flex items-center gap-2 text-md font-semibold text-fg-2 hover:text-fg-1"
                  aria-expanded={showHidden}
                >
                  <CaretDown weight="bold" className={cn("size-3.5 transition-transform", !showHidden && "-rotate-90")} />
                  Hidden <span className="font-medium text-fg-3">{hidden.length}</span>
                </button>
                {showHidden && (
                  <CardGrid className="mt-5 opacity-60 transition-opacity hover:opacity-100">
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
      {cast && (
        <AniListDialog pid={pid} characters={cast.characters} open={anilistOpen} onOpenChange={setAnilistOpen} />
      )}
    </Page>
  )
}

function CardGrid({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("grid grid-cols-[repeat(auto-fill,minmax(176px,1fr))] gap-x-5 gap-y-6", className)}>{children}</div>
}
