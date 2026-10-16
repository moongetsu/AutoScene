import { Link, useNavigate } from "@tanstack/react-router"
import { DotsThree, FilmStrip, FolderOpen, PencilSimple, Play, Television, Trash } from "@/components/icons"
import type { ProjectSummary } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
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
import { cn } from "@/lib/utils"
import { projectFacts } from "@/lib/projects"
import { useProjectIndexing } from "@/state/activity"

/** A project as its picture: the title over the footage, facts beneath it. */
export function ProjectCard({
  project: p,
  onRename,
  onDelete,
}: {
  project: ProjectSummary
  onRename: () => void
  onDelete: () => void
}) {
  const navigate = useNavigate()
  const indexing = useProjectIndexing(p.id)
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="group relative">
      <Link
        to="/p/$pid"
        params={{ pid: p.id }}
        className="block rounded-2xl transition-transform duration-[var(--dur-med)] ease-out hover:-translate-y-1 focus-visible:-translate-y-1"
      >
        <Artwork
          path={p.poster}
          className="aspect-video rounded-2xl picture-edge transition-shadow duration-[var(--dur-med)] group-hover:shadow-lift"
          imgClassName="group-hover:scale-[1.04] duration-[600ms]"
          fallback={
            <div className="flex size-full items-center justify-center artwork-fallback text-fg-disabled [&_svg]:size-10">
              {p.kind === "movie" ? <FilmStrip /> : <Television />}
            </div>
          }
        >
          <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/85 via-black/20 to-transparent" />
          <div className="absolute inset-x-0 bottom-0 p-4">
            <div className="display truncate text-3xl text-white drop-shadow-[0_2px_12px_rgb(0_0_0/0.6)]">{p.title}</div>
            {indexing ? (
              <div className="mt-2 flex items-center gap-2.5">
                <Progress value={p.mediaCount ? p.readyCount / p.mediaCount : 0} className="flex-1 bg-white/15" />
                <span className="timecode text-xs text-white/80">
                  {p.readyCount}/{p.mediaCount}
                </span>
              </div>
            ) : (
              <div className="mt-1 truncate text-sm text-white/75">{projectFacts(p)}</div>
            )}
          </div>
          {p.durationSec ? (
            <Badge tone="media" className="timecode absolute top-3 left-3">
              {formatDuration(p.durationSec)}
            </Badge>
          ) : null}
        </Artwork>
      </Link>

      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={`${p.title} options`}
          className={cn(
            "absolute top-3 right-3 flex size-8 items-center justify-center rounded-full border border-white/15 bg-black/40 text-white opacity-0 backdrop-blur-md transition-opacity",
            "group-hover:opacity-100 hover:bg-black/60 focus-visible:opacity-100 data-[state=open]:opacity-100",
          )}
        >
          <DotsThree className="size-5" weight="bold" />
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={onRename}>
            <PencilSimple /> Rename…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void commands.revealPath(p.dir)}>
            <FolderOpen /> Show project folder
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem destructive onSelect={onDelete}>
            <Trash /> Remove…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid", params: { pid: p.id } })}>
          <Play /> Open
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => void commands.revealPath(p.dir)}>
          <FolderOpen /> Show project folder
        </ContextMenuItem>
        <ContextMenuItem onSelect={onRename}>
          <PencilSimple /> Rename…
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem destructive onSelect={onDelete}>
          <Trash /> Remove…
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
