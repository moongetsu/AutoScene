import { useParams } from "@tanstack/react-router"
import { Check, CircleNotch, Clock, FolderOpen, Package, Prohibit, ShieldWarning, Warning } from "@/components/icons"
import { toast } from "sonner"
import type { ExportItem, ExportSummary, MediaItem } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { Page } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { formatBytes, formatDateTime, formatPercent } from "@/lib/format"
import { commands } from "@/lib/ipc"
import { useExports, useMediaList, useProject } from "@/lib/queries"
import { isDropFrameRate, timecode } from "@/lib/timecode"
import { useActivity } from "@/state/activity"
import { TopBar } from "@/components/ui/topbar"
import { ProjectNav } from "./ProjectNav"

function reveal(path: string) {
  commands.revealPath(path).catch(() => toast.error("That file or folder no longer exists"))
}

export function ExportsPage() {
  const { pid } = useParams({ from: "/p/$pid/exports" })
  const { data: project } = useProject(pid)
  const { data: exports, isPending } = useExports(pid)
  const { data: media } = useMediaList(pid)
  const byId = new Map(media?.map((m) => [m.id, m]))

  return (
    <Page topbar={<TopBar title={project?.title} nav={<ProjectNav pid={pid} />} />}>
      <header className="px-10 pt-10 pb-8">
        <h1 className="display text-5xl text-fg-1">Exports</h1>
        <p className="mt-2 text-md text-fg-2">Every clip is cut frame-exact and checked against the source, frame by frame.</p>
      </header>
      <div className="flex flex-col gap-5 px-10 pb-16">
        {!isPending && exports?.length === 0 && (
          <EmptyState
            icon={<Package />}
            title="No exports yet"
            description="Open an episode, mark in and out, and export the clip. Finished exports are listed here."
          />
        )}
        {exports?.map((ex) => <ExportCard key={ex.id} ex={ex} media={byId} />)}
      </div>
    </Page>
  )
}

function ExportCard({ ex, media }: { ex: ExportSummary; media: Map<string, MediaItem> }) {
  const job = useActivity((s) => s.order.map((id) => s.jobs[id]).find((j) => j.exportId === ex.id))
  const done = ex.items.filter((i) => i.status === "done").length
  const running = ex.status === "running" || ex.status === "queued"
  const unverified = ex.items.filter((i) => i.status === "done" && !i.verified).length

  return (
    <section className="overflow-hidden rounded-3xl border border-white/[0.07] bg-white/[0.03]">
      <header className="flex items-center gap-4 border-b border-white/[0.06] px-6 py-4">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-2xl bg-white/[0.06] text-fg-1 [&_svg]:size-5">
          <Package />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2.5">
            <h2 className="text-lg font-semibold text-fg-1">{ex.preset.name}</h2>
            <ExportStatusBadge ex={ex} unverified={unverified} />
          </div>
          <p className="mt-0.5 text-sm text-fg-3">
            {formatDateTime(ex.createdAt)} · {done} of {ex.items.length} {ex.items.length === 1 ? "clip" : "clips"}
          </p>
        </div>
        <Button variant="secondary" onClick={() => reveal(ex.outDir)}>
          <FolderOpen /> Show folder
        </Button>
      </header>
      {running && job && (
        <div className="flex items-center gap-4 border-b border-white/[0.06] px-6 py-3.5">
          <Progress value={job.state === "running" ? job.progress : null} className="flex-1" />
          <span className="timecode w-12 text-right text-sm text-fg-2">{formatPercent(job.progress)}</span>
        </div>
      )}
      <ul className="p-2">
        {ex.items.map((it) => (
          <ItemRow key={it.id} item={it} media={media.get(it.mediaId)} />
        ))}
      </ul>
    </section>
  )
}

function ExportStatusBadge({ ex, unverified }: { ex: ExportSummary; unverified: number }) {
  if (ex.status === "running" || ex.status === "queued") return <Badge tone="accent">{ex.status === "queued" ? "Queued" : "Exporting"}</Badge>
  if (ex.status === "cancelled") return <Badge>Cancelled</Badge>
  if (ex.status === "failed") return <Badge tone="danger">Some clips failed</Badge>
  if (unverified > 0) return <Badge tone="accent">{unverified} unverified</Badge>
  return null
}

function ItemRow({ item: it, media: m }: { item: ExportItem; media?: MediaItem }) {
  const fps = m?.fps
  const tc = (f: number) => (fps ? timecode(f, fps, isDropFrameRate(fps)) : String(f))
  return (
    <li className="group flex items-center gap-3.5 rounded-2xl px-4 py-3 transition-colors hover:bg-white/[0.04]">
      <ItemIcon item={it} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-md text-fg-1" title={it.path ?? it.name}>
          {it.name}
        </div>
        <div className="timecode mt-0.5 truncate text-xs text-fg-3">
          {m?.label ? `${m.label} · ` : ""}
          {tc(it.start)} – {tc(it.end - 1)} · {it.end - it.start} frames
          {it.bytes ? ` · ${formatBytes(it.bytes)}` : ""}
        </div>
        {it.error && <div className="mt-0.5 truncate text-sm text-danger">{it.error}</div>}
      </div>
      {it.path && it.status === "done" && (
        <Button size="sm" variant="ghost" className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100" onClick={() => reveal(it.path!)}>
          <FolderOpen /> Show
        </Button>
      )}
    </li>
  )
}

function ItemIcon({ item: it }: { item: ExportItem }) {
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
