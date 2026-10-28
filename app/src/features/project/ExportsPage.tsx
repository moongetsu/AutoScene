import { useNavigate, useParams } from "@tanstack/react-router"
import { Check, CircleNotch, Clock, Copy, FolderOpen, Package, Play, Prohibit, ShieldWarning, Warning } from "@/components/icons"
import type { ExportItem, ExportStatus, ExportSummary, MediaItem, MergedFile } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Page } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { Tooltip, TruncatedText } from "@/components/ui/tooltip"
import { formatBytes, formatDateTime, formatPercent } from "@/lib/format"
import { commands } from "@/lib/ipc"
import { copyText, reveal } from "@/lib/os"
import { firstFailure, useExports, useMediaList, useProject } from "@/lib/queries"
import { isDropFrameRate, timecode } from "@/lib/timecode"
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
        <ContextMenuItem onSelect={() => void commands.exportDefaultDir(pid).then(reveal)}>
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

function ExportCard({ pid, ex, media }: { pid: string; ex: ExportSummary; media: Map<string, MediaItem> }) {
  const job = useActivity((s) => s.order.map((id) => s.jobs[id]).find((j) => j.exportId === ex.id))
  const done = ex.items.filter((i) => i.status === "done").length
  const running = ex.status === "running" || ex.status === "queued"
  const file = ex.merged
  const unverified = file
    ? Number(file.path != null && !file.verified)
    : ex.items.filter((i) => i.status === "done" && !i.verified).length

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
              <ExportStatusBadge ex={ex} unverified={unverified} />
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
      {file && <FileRow ex={ex} file={file} />}
      <ul className="p-2">
        {ex.items.map((it) => (
          <ItemRow key={it.id} pid={pid} item={it} media={media.get(it.mediaId)} ownFile={!file} />
        ))}
      </ul>
    </section>
  )
}

function ExportStatusBadge({ ex, unverified }: { ex: ExportSummary; unverified: number }) {
  if (ex.status === "running" || ex.status === "queued") return <Badge tone="accent">{ex.status === "queued" ? "Queued" : "Exporting"}</Badge>
  if (ex.status === "cancelled") return <Badge>Cancelled</Badge>
  if (ex.status === "failed" && ex.merged?.error) return <Badge tone="danger">File not made</Badge>
  if (ex.status === "failed") return <Badge tone="danger">Some clips failed</Badge>
  if (unverified > 0) return <Badge tone="accent">{unverified} unverified</Badge>
  return null
}

/** The one file an export joins its clips into: there once every clip is rendered. */
function FileRow({ ex, file }: { ex: ExportSummary; file: MergedFile }) {
  const status: ExportStatus = file.path ? "done" : file.error || ex.status === "done" ? "failed" : ex.status
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
