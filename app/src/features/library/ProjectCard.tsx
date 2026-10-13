import { useState } from "react"
import { Link } from "@tanstack/react-router"
import { Film, FolderOpen, MoreHorizontal, Pencil, Trash2, Tv } from "lucide-react"
import type { ProjectSummary } from "@/bindings"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Progress } from "@/components/ui/progress"
import { formatDuration } from "@/lib/format"
import { commands } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { cn } from "@/lib/utils"
import { useProjectIndexing } from "@/state/activity"

const CONTENT = { live: "Live action", anime: "Anime", mixed: "Mixed" } as const

export function ProjectCard({
  project: p,
  onRename,
  onDelete,
}: {
  project: ProjectSummary
  onRename: () => void
  onDelete: () => void
}) {
  const [posterFailed, setPosterFailed] = useState(false)
  const indexing = useProjectIndexing(p.id)
  const unit = p.kind === "movie" ? (p.mediaCount === 1 ? "file" : "files") : p.mediaCount === 1 ? "episode" : "episodes"

  return (
    <div className="group relative">
      <Link
        to="/p/$pid/episodes"
        params={{ pid: p.id }}
        className={cn(
          "block overflow-hidden rounded-lg border border-hairline bg-panel shadow-1 transition-[border-color,transform] duration-[var(--dur-med)]",
          "hover:border-strong focus-visible:border-accent-line",
        )}
      >
        <div className="relative aspect-video overflow-hidden bg-inset">
          {p.poster && !posterFailed ? (
            <img
              src={assetUrl(p.poster)}
              alt=""
              draggable={false}
              onError={() => setPosterFailed(true)}
              className="size-full object-cover transition-transform duration-[var(--dur-slow)] ease-out group-hover:scale-[1.02]"
            />
          ) : (
            <div className="flex size-full items-center justify-center text-fg-disabled [&_svg]:size-7">
              {p.kind === "movie" ? <Film /> : <Tv />}
            </div>
          )}
          {/* Legibility for the meta line without tinting the footage. */}
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-scrim to-transparent" />
          <span className="timecode absolute right-2 bottom-1.5 text-2xs text-on-media">
            {p.durationSec ? formatDuration(p.durationSec) : ""}
          </span>
        </div>
        <div className="px-3.5 pt-2.5 pb-3">
          <div className="truncate text-sm font-semibold text-fg-1">{p.title}</div>
          <div className="mt-0.5 flex items-center gap-1.5 text-xs text-fg-3">
            <span>
              {p.kind === "movie" ? "Movie" : "Series"} · {CONTENT[p.content]}
            </span>
            <span aria-hidden>·</span>
            <span>
              {p.mediaCount} {unit}
            </span>
          </div>
          {indexing && (
            <div className="mt-2.5 flex items-center gap-2">
              <Progress value={p.mediaCount ? p.readyCount / p.mediaCount : 0} className="flex-1" />
              <span className="timecode text-2xs text-fg-3">
                {p.readyCount}/{p.mediaCount}
              </span>
            </div>
          )}
        </div>
      </Link>

      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={`${p.title} options`}
          className={cn(
            "absolute top-2 right-2 flex size-7 items-center justify-center rounded-md border border-soft bg-overlay text-fg-2 opacity-0 shadow-1 transition-opacity",
            "group-hover:opacity-100 hover:text-fg-1 focus-visible:opacity-100 data-[state=open]:opacity-100",
          )}
        >
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={onRename}>
            <Pencil /> Rename…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void commands.revealPath(p.dir)}>
            <FolderOpen /> Show project folder
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem destructive onSelect={onDelete}>
            <Trash2 /> Remove…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
