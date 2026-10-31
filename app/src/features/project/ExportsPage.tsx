import { useId, useState } from "react"
import { useNavigate, useParams } from "@tanstack/react-router"
import { CaretDown, Check, CircleNotch, Clock, Copy, FolderOpen, Package, Play, Prohibit, ShieldWarning, Warning } from "@/components/icons"
import { toast } from "sonner"
import type { ExportItem, ExportStatus, ExportSummary, JobInfo, MediaItem, MergedFile } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Page } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { Tooltip, TruncatedText } from "@/components/ui/tooltip"
import { formatBytes, formatDateTime, formatDuration, formatPercent } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { copyText, reveal } from "@/lib/os"
import { firstFailure, useExports, useMediaList, useProject } from "@/lib/queries"
import { fpsValue, isDropFrameRate, timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { useActivity } from "@/state/activity"
import { TopBar } from "@/components/ui/topbar"
import { ProjectLoadError } from "./ProjectLoadError"
import { ProjectNav } from "./ProjectNav"

export function ExportsPage() {
  const { pid } = useParams({ from: "/p/$pid/exports" })
  const projectQuery = useProject(pid)
  const exportsQuery = useExports(pid)
  const mediaQuery = useMediaList(pid)
  const { data: project } = projectQuery
  const { data: exports } = exportsQuery
  const { data: media } = mediaQuery
  const byId = new Map(media?.map((m) => [m.id, m]))
  const failure = firstFailure(projectQuery, exportsQuery, mediaQuery)

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
        <ContextMenuItem
          onSelect={() =>
            void commands
              .exportDefaultDir(pid)
              .then(reveal)
              .catch((e) => toast.error("Couldn't show it", { description: errorMessage(e) }))
          }
        >
          <FolderOpen /> Show the exports folder
        </ContextMenuItem>
      }
    >
      <header className="px-10 pt-10 pb-8">
        <h1 className="display text-5xl text-fg-1">Exports</h1>
        <p className="mt-2 text-md text-fg-2">Every clip is cut frame-exact and checked against the source, frame by frame.</p>
      </header>
      <div className="flex flex-col gap-5 px-10 pb-16">
        {exports?.length === 0 && (
          <EmptyState
            icon={<Package />}
            title="No exports yet"
            description="Open an episode, mark in and out, and export the clip. Finished exports are listed here."
          />
        )}
        {exports?.map((ex) => <ExportCard key={ex.id} pid={pid} ex={ex} media={byId} />)}
      </div>
    </Page>
  )
}

/** Above this many clips, a finished export's list starts folded: the cards stay short. */
const FOLD_OVER = 4

function ExportCard({ pid, ex, media }: { pid: string; ex: ExportSummary; media: Map<string, MediaItem> }) {
  const job = useActivity((s) => s.order.map((id) => s.jobs[id]).find((j) => j.exportId === ex.id))
  const done = ex.items.filter((i) => i.status === "done").length
  const status = exportStatus(ex, job)
  const running = status === "running" || status === "queued"
  const file = ex.merged
  const unverified = file
    ? Number(file.path != null && !file.verified)
    : ex.items.filter((i) => i.status === "done" && !i.verified).length
  // Open while it runs, to watch the clips get done; folded once it's history.
  const foldable = ex.items.length > FOLD_OVER
  const [unfolded, setUnfolded] = useState(running)
  const open = !foldable || unfolded
  // Folded, the clips that failed still show: their errors aren't hidden.
  const listed = open ? ex.items : ex.items.filter((i) => i.status === "failed" || i.error != null)
  const seconds = open ? null : runTime(ex.items, media)
  const listId = useId()

  return (
    <section className="overflow-hidden rounded-3xl border border-white/[0.07] bg-white/[0.03]">
      <ContextArea
        menu={
          <>
            <ContextMenuItem onSelect={() => reveal(ex.outDir)}>
              <FolderOpen /> Show folder
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => void copyText(ex.outDir, "folder path")}>
              <Copy /> Copy folder path
            </ContextMenuItem>
          </>
        }
      >
        <header className="flex items-center gap-4 border-b border-white/[0.06] px-6 py-4">
          <span className="flex size-11 shrink-0 items-center justify-center rounded-2xl bg-white/[0.06] text-fg-1 [&_svg]:size-5">
            <Package />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2.5">
              <h2 className="truncate text-lg font-semibold text-fg-1">{ex.label ?? ex.preset.name}</h2>
              <ExportStatusBadge ex={ex} status={status} unverified={unverified} />
            </div>
            <p className="mt-0.5 text-sm text-fg-3">
              {ex.label && `${ex.preset.name} · `}
              {formatDateTime(ex.createdAt)} · {done} of {ex.items.length} {ex.items.length === 1 ? "clip" : "clips"}
              {file && " in one file"}
            </p>
          </div>
          <Button variant="secondary" onClick={() => reveal(ex.outDir)}>
            <FolderOpen /> Show folder
          </Button>
        </header>
      </ContextArea>
      {running && job && (
        <div className="flex items-center gap-4 border-b border-white/[0.06] px-6 py-3.5">
          <Progress value={job.state === "running" ? job.progress : null} className="flex-1" />
          <span className="timecode w-12 text-right text-sm text-fg-2">{formatPercent(job.progress)}</span>
        </div>
      )}
      {file && <FileRow ex={ex} status={status} file={file} />}
      <div className="p-2">
        {foldable && (
          <button
            type="button"
            aria-expanded={open}
            aria-controls={listId}
            onClick={() => setUnfolded(!open)}
            className="flex w-full items-center gap-2.5 rounded-2xl px-4 py-2.5 text-left text-sm font-medium text-fg-2 transition-colors hover:bg-white/[0.04] hover:text-fg-1"
          >
            <CaretDown weight="bold" className={cn("size-3.5 transition-transform duration-[var(--dur-fast)]", !open && "-rotate-90")} />
            <span>
              {open ? "Hide clips" : `Show ${listed.length > 0 ? "all " : ""}${ex.items.length} clips`}
              {seconds != null && <span className="font-normal text-fg-3"> · {formatDuration(seconds)}</span>}
            </span>
          </button>
        )}
        <ul id={listId}>
          {listed.map((it) => (
            <ItemRow key={it.id} pid={pid} item={it} media={media.get(it.mediaId)} ownFile={!file} />
          ))}
        </ul>
      </div>
    </section>
  )
}

/** How long the clips run in all, in seconds; `null` while a clip's frame rate isn't known. */
function runTime(items: ExportItem[], media: Map<string, MediaItem>): number | null {
  let seconds = 0
  for (const it of items) {
    const fps = media.get(it.mediaId)?.fps
    if (!fps) return null
    seconds += (it.end - it.start) / fpsValue(fps)
  }
  return seconds
}

/**
 * Where an export stands. Its job can end before its record says so (the
 * list is read again a moment later, see `lib/events.ts`): a cancelled or
 * failed job's card says so at once rather than going on showing it queued
 * or running. A job done waits for the record, which knows what was made.
 */
function exportStatus(ex: ExportSummary, job: JobInfo | undefined): ExportStatus {
  const unfinished = ex.status === "running" || ex.status === "queued"
  const ended = job?.state === "cancelled" || job?.state === "failed"
  return unfinished && ended ? job.state : ex.status
}

function ExportStatusBadge({ ex, status, unverified }: { ex: ExportSummary; status: ExportStatus; unverified: number }) {
  if (status === "running" || status === "queued") return <Badge tone="accent">{status === "queued" ? "Queued" : "Exporting"}</Badge>
  if (status === "cancelled") return <Badge>Cancelled</Badge>
  if (status === "failed" && ex.merged?.error) return <Badge tone="danger">File not made</Badge>
  if (status === "failed") return <Badge tone="danger">Some clips failed</Badge>
  if (unverified > 0) return <Badge tone="accent">{unverified} unverified</Badge>
  return null
}

/** The one file an export joins its clips into: there once every clip is rendered. */
function FileRow({ ex, status: ofExport, file }: { ex: ExportSummary; status: ExportStatus; file: MergedFile }) {
  const status: ExportStatus = file.path ? "done" : file.error || ofExport === "done" ? "failed" : ofExport
  const chapters = ex.items.filter((i) => i.path != null && i.path === file.path).length
  const rendered = ex.items.every((i) => i.status !== "queued" && i.status !== "running")
  const what =
    status === "done"
      ? `${chapters} ${chapters === 1 ? "chapter" : "chapters"}${file.bytes ? ` · ${formatBytes(file.bytes)}` : ""}`
      : status === "cancelled"
        ? "Not made"
        : status === "failed"
          ? "Couldn't be made"
          : rendered
            ? "Joining the clips…"
            : "Made once every clip is rendered"
  return (
    <ContextArea
      menu={
        <>
          {file.path && (
            <ContextMenuItem onSelect={() => reveal(file.path!)}>
              <FolderOpen /> Show in folder
            </ContextMenuItem>
          )}
          {file.path && (
            <ContextMenuItem onSelect={() => void copyText(file.path!, "file path")}>
              <Copy /> Copy file path
            </ContextMenuItem>
          )}
          {file.error && (
            <ContextMenuItem onSelect={() => void copyText(file.error!, "error")}>
              <Copy /> Copy the error
            </ContextMenuItem>
          )}
        </>
      }
    >
      <div className="flex items-center gap-3.5 border-b border-white/[0.06] px-6 py-3.5">
        <ItemIcon item={{ status, verified: file.verified }} />
        <div className="min-w-0 flex-1">
          <Tooltip label={file.path} side="top" delay={600} disabled={!file.path}>
            <div className="truncate text-md font-medium text-fg-1" data-selectable>
              {/* The file as written (Bit-Perfect may make an .mkv), else as planned. */}
              {file.path ? file.path.split(/[\\/]/).pop() : `${file.name}.${ex.preset.container}`}
            </div>
          </Tooltip>
          <div className="mt-0.5 truncate text-xs text-fg-3">{what}</div>
          {file.error && <div className="mt-0.5 truncate text-sm text-danger">{file.error}</div>}
        </div>
        {file.path && (
          <Button size="sm" variant="secondary" onClick={() => reveal(file.path!)}>
            <FolderOpen /> Show
          </Button>
        )}
      </div>
    </ContextArea>
  )
}

function ItemRow({ pid, item: it, media: m, ownFile }: { pid: string; item: ExportItem; media?: MediaItem; ownFile: boolean }) {
  const navigate = useNavigate()
  const fps = m?.fps
  const tc = (f: number) => (fps ? timecode(f, fps, isDropFrameRate(fps)) : String(f))
  return (
    <ContextArea
      menu={
        <>
          <ContextMenuItem
            disabled={!m}
            onSelect={() => void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: it.mediaId }, search: { f: it.start } })}
          >
            <Play /> Watch the source
          </ContextMenuItem>
          <ContextMenuSeparator />
          {ownFile && it.path && (
            <ContextMenuItem onSelect={() => reveal(it.path!)}>
              <FolderOpen /> Show in folder
            </ContextMenuItem>
          )}
          {it.path && (
            <ContextMenuItem onSelect={() => void copyText(it.path!, "file path")}>
              <Copy /> Copy file path
            </ContextMenuItem>
          )}
          {fps && (
            <ContextMenuItem onSelect={() => void copyText(`${tc(it.start)} - ${tc(it.end - 1)}`, "timecodes")}>
              <Copy /> Copy timecodes
            </ContextMenuItem>
          )}
          {it.error && (
            <ContextMenuItem onSelect={() => void copyText(it.error!, "error")}>
              <Copy /> Copy the error
            </ContextMenuItem>
          )}
        </>
      }
    >
      <li className="group flex items-center gap-3.5 rounded-2xl px-4 py-3 transition-colors hover:bg-white/[0.04]">
        <ItemIcon item={it} />
        <div className="min-w-0 flex-1">
          {it.path ? (
            <Tooltip label={it.path} side="top" delay={600}>
              <div className="truncate text-md text-fg-1">{it.name}</div>
            </Tooltip>
          ) : (
            <TruncatedText className="text-md text-fg-1">{it.name}</TruncatedText>
          )}
          <div className="timecode mt-0.5 truncate text-xs text-fg-3">
            {m?.label ? `${m.label} · ` : ""}
            {tc(it.start)} – {tc(it.end - 1)} · {it.end - it.start} frames
            {it.bytes ? ` · ${formatBytes(it.bytes)}` : ""}
          </div>
          {it.error && <div className="mt-0.5 truncate text-sm text-danger">{it.error}</div>}
        </div>
        {ownFile && it.path && it.status === "done" && (
          <Button size="sm" variant="ghost" className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100" onClick={() => reveal(it.path!)}>
            <FolderOpen /> Show
          </Button>
        )}
      </li>
    </ContextArea>
  )
}

function ItemIcon({ item: it }: { item: Pick<ExportItem, "status" | "verified"> }) {
  const box = "flex size-8 shrink-0 items-center justify-center rounded-full [&_svg]:size-4"
  switch (it.status) {
    case "done":
      return it.verified ? (
        <Tooltip label="Verified: every frame matches the source">
          <span className={`${box} bg-ok/15 text-ok`}>
            <Check weight="bold" />
          </span>
        </Tooltip>
      ) : (
        <Tooltip label="Exported, but the frames couldn't be verified">
          <span className={`${box} bg-accent-subtle text-accent`}>
            <ShieldWarning />
          </span>
        </Tooltip>
      )
    case "running":
      return (
        <span className={`${box} text-accent`}>
          <CircleNotch className="animate-spin" />
        </span>
      )
    case "failed":
      return (
        <span className={`${box} bg-danger-subtle text-danger`}>
          <Warning />
        </span>
      )
    case "cancelled":
      return (
        <span className={`${box} text-fg-3`}>
          <Prohibit />
        </span>
      )
    default:
      return (
        <span className={`${box} text-fg-3`}>
          <Clock />
        </span>
      )
  }
}
