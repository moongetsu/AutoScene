import { useMemo, useState } from "react"
import { useNavigate, useParams } from "@tanstack/react-router"
import {
  ArrowCounterClockwise,
  DotsThree,
  FilePlus,
  FolderOpen,
  FolderPlus,
  MagnifyingGlass,
  Play,
  Scissors,
  Trash,
  UserFocus,
} from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Page } from "@/components/ui/page"
import { Select } from "@/components/ui/select"
import { TopBar } from "@/components/ui/topbar"
import { formatBytes, formatDuration } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import {
  audioLabel,
  audioOption,
  codecLabel,
  currentAudio,
  fileStem,
  formatFlags,
  pickFolder,
  pickVideoFiles,
  releaseTitle,
} from "@/lib/media"
import { addToProject, projectFacts } from "@/lib/projects"
import { firstFailure, useMediaList, useProject } from "@/lib/queries"
import { formatFps } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { MissingBanner } from "./MissingBanner"
import { ProjectLoadError } from "./ProjectLoadError"
import { ProjectNav } from "./ProjectNav"
import { locateFile } from "./relink"
import { RemoveMediaDialog } from "./RemoveMediaDialog"
import { StatusCell } from "./StatusCell"

/** Every file of a project with its format, audio track and analysis state. */
export function EpisodesPage() {
  const { pid } = useParams({ from: "/p/$pid/episodes" })
  const projectQuery = useProject(pid)
  const mediaQuery = useMediaList(pid)
  const { data: project } = projectQuery
  const { data: media, isPending } = mediaQuery
  const navigate = useNavigate()
  const [removing, setRemoving] = useState<MediaItem | null>(null)
  const nameCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const m of media ?? []) counts.set(releaseTitle(m.fileName), (counts.get(releaseTitle(m.fileName)) ?? 0) + 1)
    return counts
  }, [media])
  const movie = project?.kind === "movie"
  const total = media?.reduce((s, m) => s + (m.durationSec ?? 0), 0) ?? 0
  const failure = firstFailure(projectQuery, mediaQuery)

  if (failure) {
    return (
      <Page topbar={<TopBar title={project?.title} nav={<ProjectNav pid={pid} />} />}>
        <ProjectLoadError error={failure.error} onRetry={failure.retry} />
      </Page>
    )
  }

  return (
    <Page topbar={<TopBar title={project?.title} nav={<ProjectNav pid={pid} />} />}>
      <header className="flex flex-wrap items-end justify-between gap-6 px-10 pt-10 pb-8">
        <div>
          <h1 className="display text-5xl text-fg-1">{movie ? "Files" : "Episodes"}</h1>
          {project && (
            <p className="mt-2 text-md text-fg-2">
              {projectFacts(project)}
              {total > 0 && ` · ${formatDuration(total)}`}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2.5">
          <Button size="lg" variant="secondary" onClick={() => void pickVideoFiles().then((p) => addToProject(pid, p))}>
            <FilePlus /> Add files
          </Button>
          <Button
            size="lg"
            variant="primary"
            onClick={() => void pickFolder().then((p) => (p ? addToProject(pid, [p]) : undefined))}
          >
            <FolderPlus /> Add folder
          </Button>
        </div>
      </header>

      <div className="px-10 pb-16">
        <MissingBanner pid={pid} media={media} movie={movie} className="mb-6" />
        {!isPending && media?.length === 0 ? (
          <EmptyState
            icon={<FolderPlus />}
            title="No videos yet"
            description="Drop a season folder or video files anywhere in this window, or add them here."
          />
        ) : (
          // Details fold away as the window narrows; picture, name and state always stay.
          <ul className="@container flex flex-col gap-1.5">
            {isPending
              ? Array.from({ length: 3 }, (_, i) => <li key={i} className="skeleton h-[120px] rounded-2xl" />)
              : media!.map((m) => (
                  <EpisodeRow
                    key={m.id}
                    media={m}
                    ambiguous={(nameCounts.get(releaseTitle(m.fileName)) ?? 0) > 1}
                    projectId={pid}
                    faces={project?.content !== "anime"}
                    onOpen={() => void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: m.id } })}
                    onRemove={() => setRemoving(m)}
                  />
                ))}
          </ul>
        )}
      </div>
      <RemoveMediaDialog projectId={pid} media={removing} onOpenChange={(o) => !o && setRemoving(null)} />
    </Page>
  )
}

function EpisodeRow({
  media: m,
  ambiguous,
  projectId,
  faces,
  onOpen,
  onRemove,
}: {
  media: MediaItem
  /** Another row has the same clean name: show the file name too. */
  ambiguous: boolean
  projectId: string
  /** The project finds characters (live action). */
  faces: boolean
  onOpen: () => void
  onRemove: () => void
}) {
  const redo = (stage: "shots" | "faces") =>
    void commands
      .mediaAnalyze(projectId, m.id, stage)
      .catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))
  const v = m.info?.video
  const ready = m.status === "ready"
  const audio = m.info?.audio ?? []
  const flags = formatFlags(m)

  const setAudio = (value: string) => {
    commands
      .mediaSetAudio(projectId, m.id, Number(value))
      .catch((e) => toast.error("Couldn't change the audio track", { description: errorMessage(e) }))
  }

  return (
    <li
      onClick={ready ? onOpen : undefined}
      tabIndex={ready ? 0 : undefined}
      aria-label={ready ? `Open ${m.label ? `${m.label} ` : ""}${releaseTitle(m.fileName)}` : undefined}
      onKeyDown={(e) => {
        if (ready && e.key === "Enter" && e.target === e.currentTarget) onOpen()
      }}
      className={cn(
        "group grid grid-cols-[176px_minmax(0,1fr)_auto] items-center gap-5 rounded-2xl p-2.5 pr-4 transition-colors focus-visible:bg-white/[0.04] focus-visible:outline-none @3xl:grid-cols-[192px_minmax(0,1fr)_200px_190px_auto]",
        ready && "hover:bg-white/[0.04]",
      )}
    >
      <Artwork path={m.posterSmall ?? m.poster} className="aspect-video rounded-xl picture-edge" imgClassName={cn(!ready && "opacity-40 grayscale")}>
        {ready && (
          <span className="absolute inset-0 flex items-center justify-center bg-black/35 opacity-0 transition-opacity group-hover:opacity-100">
            <span className="flex size-10 items-center justify-center rounded-full border border-white/20 bg-black/40 text-white backdrop-blur-md [&_svg]:size-4">
              <Play weight="fill" />
            </span>
          </span>
        )}
        {m.durationSec ? (
          <span className="timecode absolute right-2 bottom-1.5 text-2xs text-white/85 drop-shadow-[0_1px_3px_rgb(0_0_0/0.8)]">
            {formatDuration(m.durationSec)}
          </span>
        ) : null}
      </Artwork>

      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2.5">
          {m.label && (
            <Badge tone="neutral" className="timecode shrink-0 text-fg-1">
              {m.label}
            </Badge>
          )}
          <span className="truncate text-lg font-medium text-fg-1" title={m.fileName}>
            {releaseTitle(m.fileName)}
          </span>
        </div>
        <div className="mt-1.5 truncate text-sm text-fg-3" title={m.fileName}>
          {ambiguous && <>{fileStem(m.fileName)} · </>}
          {formatBytes(m.sizeBytes ?? 0)}
          {v && (
            <>
              {" "}
              · {v.width}×{v.height} · {m.fps ? formatFps(m.fps) : "–"} · {codecLabel(v.codec)}
            </>
          )}
        </div>
        {flags.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {flags.map((f) => (
              <Badge key={f} tone="outline" className="h-5 px-2 text-2xs">
                {f}
              </Badge>
            ))}
          </div>
        )}
        {/* Narrow windows: the state under the name. */}
        <div className="mt-2.5 @3xl:hidden" onClick={(e) => e.stopPropagation()}>
          <StatusCell media={m} projectId={projectId} faces={faces} />
        </div>
      </div>

      <div className="hidden @3xl:block" onClick={(e) => e.stopPropagation()}>
        {audio.length > 1 ? (
          <Select
            size="sm"
            aria-label="Audio track"
            className="w-full"
            value={String(currentAudio(m))}
            onChange={setAudio}
            options={audio.map(audioOption)}
          />
        ) : (
          <span className="block truncate text-sm text-fg-3">{!m.info ? "–" : audio[0] ? audioLabel(audio[0]) : "No audio"}</span>
        )}
      </div>

      <div className="hidden @3xl:block" onClick={(e) => e.stopPropagation()}>
        <StatusCell media={m} projectId={projectId} faces={faces} />
      </div>

      <div onClick={(e) => e.stopPropagation()}>
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label={`${m.fileName} options`}
            className="flex size-9 items-center justify-center rounded-full text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-white/[0.08] hover:text-fg-1 focus-visible:opacity-100 data-[state=open]:opacity-100"
          >
            <DotsThree className="size-5" weight="bold" />
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem disabled={!ready} onSelect={onOpen}>
              <Play /> Open
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void commands.revealPath(m.path).catch(() => toast.error("File not found"))}>
              <FolderOpen /> Show in folder
            </DropdownMenuItem>
            {m.status === "missing" && (
              <DropdownMenuItem onSelect={() => void locateFile(projectId, m)}>
                <MagnifyingGlass /> Locate file…
              </DropdownMenuItem>
            )}
            {m.status === "failed" && (
              <DropdownMenuItem onSelect={() => void commands.mediaRetry(projectId, m.id)}>
                <ArrowCounterClockwise /> Index again
              </DropdownMenuItem>
            )}
            {ready && (
              <DropdownMenuItem onSelect={() => redo("shots")}>
                <Scissors /> Find shots again
              </DropdownMenuItem>
            )}
            {ready && faces && m.shots === "ready" && (
              <DropdownMenuItem onSelect={() => redo("faces")}>
                <UserFocus /> Find characters again
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem destructive onSelect={onRemove}>
              <Trash /> Remove from project…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </li>
  )
}
