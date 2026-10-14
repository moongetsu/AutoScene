import { useMemo, useState } from "react"
import { useNavigate, useParams } from "@tanstack/react-router"
import { FilePlus2, FolderOpen, FolderPlus, MoreHorizontal, Play, RotateCcw, ScanFace, Scissors, Trash2 } from "lucide-react"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
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
import { Page, PageHeader } from "@/components/ui/page"
import { Select } from "@/components/ui/select"
import { formatBytes, formatDuration } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import {
  assetUrl,
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
import { useMediaList, useProject } from "@/lib/queries"
import { formatFps } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { StatusCell } from "./StatusCell"

const CONTENT = { live: "Live action", anime: "Anime", mixed: "Mixed" } as const

export function EpisodesPage() {
  const { pid } = useParams({ from: "/p/$pid/episodes" })
  const { data: project } = useProject(pid)
  const { data: media, isPending } = useMediaList(pid)
  const navigate = useNavigate()
  const nameCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const m of media ?? []) counts.set(releaseTitle(m.fileName), (counts.get(releaseTitle(m.fileName)) ?? 0) + 1)
    return counts
  }, [media])

  const add = async (paths: string[]) => {
    if (!paths.length) return
    try {
      const n = await commands.mediaAdd(pid, paths)
      toast(n === 0 ? "Those files are already in this project" : `Added ${n} ${n === 1 ? "file" : "files"}`)
    } catch (e) {
      toast.error("Couldn't add files", { description: errorMessage(e) })
    }
  }

  const isMovie = project?.kind === "movie"
  const total = media?.reduce((s, m) => s + (m.durationSec ?? 0), 0) ?? 0

  return (
    <Page>
      <PageHeader
        title={project?.title ?? " "}
        description={
          project && (
            <>
              {isMovie ? "Movie" : "Series"} · {CONTENT[project.content]} · {media?.length ?? 0}{" "}
              {isMovie ? (media?.length === 1 ? "file" : "files") : media?.length === 1 ? "episode" : "episodes"}
              {total > 0 && <> · {formatDuration(total)}</>}
            </>
          )
        }
        actions={
          <>
            <Button size="md" variant="ghost" onClick={() => void pickVideoFiles().then(add)}>
              <FilePlus2 /> Add files
            </Button>
            <Button size="md" onClick={() => void pickFolder().then((p) => (p ? add([p]) : undefined))}>
              <FolderPlus /> Add folder
            </Button>
          </>
        }
      />
      <div className="px-8 pb-10">
        {!isPending && media?.length === 0 ? (
          <EmptyState
            icon={<FolderPlus />}
            title="No videos yet"
            description="Drop a season folder or video files anywhere in this window, or add them here."
            className="rounded-xl border border-hairline bg-panel"
          />
        ) : (
          // Columns fold away as the window narrows; the name and the row menu always stay.
          <div className="@container overflow-hidden rounded-lg border border-hairline bg-panel shadow-1">
            <table className="w-full table-fixed text-sm">
              <thead>
                <tr className="border-b border-hairline text-left">
                  <th className="eyebrow w-[160px] py-2.5 pl-4 font-medium">{isMovie ? "File" : "Episode"}</th>
                  <th className="eyebrow py-2.5 font-medium">Name</th>
                  <th className="eyebrow hidden w-[84px] py-2.5 font-medium @4xl:table-cell">Length</th>
                  <th className="eyebrow hidden w-[220px] py-2.5 font-medium @5xl:table-cell">Format</th>
                  <th className="eyebrow w-[176px] py-2.5 font-medium">Audio</th>
                  <th className="eyebrow w-[176px] py-2.5 font-medium">Status</th>
                  <th className="w-11" />
                </tr>
              </thead>
              <tbody>
                {isPending
                  ? Array.from({ length: 3 }, (_, i) => (
                      <tr key={i} className="border-b border-hairline last:border-0">
                        <td colSpan={7} className="px-4 py-3">
                          <div className="skeleton h-12 rounded-sm" />
                        </td>
                      </tr>
                    ))
                  : media!.map((m) => (
                      <EpisodeRow
                        key={m.id}
                        media={m}
                        ambiguous={(nameCounts.get(releaseTitle(m.fileName)) ?? 0) > 1}
                        projectId={pid}
                        faces={project?.content !== "anime"}
                        onOpen={() => void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: m.id } })}
                      />
                    ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </Page>
  )
}

function EpisodeRow({
  media: m,
  ambiguous,
  projectId,
  faces,
  onOpen,
}: {
  media: MediaItem
  /** Another row has the same clean name: show the file name too. */
  ambiguous: boolean
  projectId: string
  /** The project finds characters (live action). */
  faces: boolean
  onOpen: () => void
}) {
  const redo = (stage: "shots" | "faces") =>
    void commands
      .mediaAnalyze(projectId, m.id, stage)
      .catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))
  const [posterFailed, setPosterFailed] = useState(false)
  const v = m.info?.video
  const ready = m.status === "ready"
  const audio = m.info?.audio ?? []

  const setAudio = (value: string) => {
    commands
      .mediaSetAudio(projectId, m.id, Number(value))
      .catch((e) => toast.error("Couldn't change the audio track", { description: errorMessage(e) }))
  }

  return (
    <tr
      onClick={ready ? onOpen : undefined}
      tabIndex={ready ? 0 : undefined}
      aria-label={ready ? `Open ${m.label ? `${m.label} ` : ""}${releaseTitle(m.fileName)}` : undefined}
      onKeyDown={(e) => {
        if (ready && e.key === "Enter" && e.target === e.currentTarget) onOpen()
      }}
      className={cn(
        "group border-b border-hairline transition-colors last:border-0 focus-visible:bg-hover focus-visible:outline-none",
        ready ? "cursor-pointer hover:bg-hover" : "",
      )}
    >
      <td className="py-2 pr-3 pl-4">
        <div className="flex min-w-0 items-center gap-3">
          <div className="relative aspect-video w-[72px] shrink-0 overflow-hidden rounded-xs bg-inset">
            {m.poster && !posterFailed && (
              <img
                src={assetUrl(m.poster)}
                alt=""
                draggable={false}
                onError={() => setPosterFailed(true)}
                className="size-full object-cover"
              />
            )}
            {ready && (
              <span className="absolute inset-0 flex items-center justify-center bg-scrim-media opacity-0 transition-opacity group-hover:opacity-100">
                <Play className="size-4 fill-on-media text-on-media" />
              </span>
            )}
          </div>
          <span className={cn("timecode truncate text-xs", m.label ? "text-fg-1" : "text-fg-3")} title={m.label || undefined}>
            {m.label || "—"}
          </span>
        </div>
      </td>
      <td className="max-w-0 py-2 pr-4">
        <div className="truncate text-fg-1" title={m.fileName}>
          {releaseTitle(m.fileName)}
        </div>
        <div className="timecode truncate text-2xs text-fg-3" title={m.fileName}>
          {ambiguous && <>{fileStem(m.fileName)} · </>}
          {formatBytes(m.sizeBytes ?? 0)}
          {m.durationSec ? <span className="@4xl:hidden"> · {formatDuration(m.durationSec)}</span> : null}
        </div>
      </td>
      <td className="timecode hidden py-2 text-xs text-fg-2 @4xl:table-cell">
        {m.durationSec ? formatDuration(m.durationSec) : "–"}
      </td>
      <td className="hidden py-2 @5xl:table-cell">
        {v ? (
          <div className="flex flex-col gap-1">
            <span className="timecode text-xs text-fg-2">
              {v.width}×{v.height} · {m.fps ? formatFps(m.fps) : "–"} · {codecLabel(v.codec)}
            </span>
            <span className="flex flex-wrap gap-1">
              {formatFlags(m).map((f) => (
                <Badge key={f} tone="outline">
                  {f}
                </Badge>
              ))}
            </span>
          </div>
        ) : (
          <span className="text-xs text-fg-3">–</span>
        )}
      </td>
      <td className="py-2 pr-3" onClick={(e) => e.stopPropagation()}>
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
          <span className="block truncate text-xs text-fg-3">
            {!m.info ? "–" : audio[0] ? audioLabel(audio[0]) : "No audio"}
          </span>
        )}
      </td>
      <td className="py-2">
        <StatusCell media={m} projectId={projectId} faces={faces} />
      </td>
      <td className="py-2 pr-2" onClick={(e) => e.stopPropagation()}>
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label={`${m.fileName} options`}
            className="flex size-7 items-center justify-center rounded-md text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-fg-1 focus-visible:opacity-100 data-[state=open]:opacity-100"
          >
            <MoreHorizontal className="size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem disabled={!ready} onSelect={onOpen}>
              <Play /> Open
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void commands.revealPath(m.path).catch(() => toast.error("File not found"))}>
              <FolderOpen /> Show in folder
            </DropdownMenuItem>
            {(m.status === "failed" || m.status === "missing") && (
              <DropdownMenuItem onSelect={() => void commands.mediaRetry(projectId, m.id)}>
                <RotateCcw /> Index again
              </DropdownMenuItem>
            )}
            {ready && (
              <DropdownMenuItem onSelect={() => redo("shots")}>
                <Scissors /> Find shots again
              </DropdownMenuItem>
            )}
            {ready && faces && m.shots === "ready" && (
              <DropdownMenuItem onSelect={() => redo("faces")}>
                <ScanFace /> Find characters again
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              destructive
              onSelect={() =>
                void commands
                  .mediaRemove(projectId, m.id)
                  .catch((e) => toast.error("Couldn't remove", { description: errorMessage(e) }))
              }
            >
              <Trash2 /> Remove from project
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </td>
    </tr>
  )
}
