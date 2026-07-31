import { useMemo, useState } from "react"
import { motion } from "motion/react"
import {
  CheckCircle2,
  Clock3,
  Film,
  FolderOpen,
  Loader2,
  ScanFace,
  Search,
  AlertTriangle,
} from "lucide-react"
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import { DeleteProjectButton } from "@/components/dashboard/DeleteProjectButton"
import { Input } from "@/components/ui/input"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { formatDuration, type Project } from "@/lib/api"
import {
  formatAbsoluteStamp,
  formatRelativeTime,
  mediaLabel,
} from "@/lib/format"
import { friendlyBackend, friendlyError, friendlyModel, friendlyProjectStatus, sanitizeStage } from "@/lib/plainCopy"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { cn } from "@/lib/utils"

type ProjectLoaderProps = {
  projects: Project[]
  /** False until first project list fetch finishes. */
  loading?: boolean
  activeId: string | null
  onOpen: (project: Project) => void
  onDeleted?: (id: string) => void | Promise<void>
  disabled?: boolean
}

const statusRank: Record<string, number> = {
  cutting: 0,
  scanning: 0,
  paused: 0,
  exporting: 1,
  error: 2,
  ready: 3,
  idle: 4,
}

function statusTone(status: Project["status"]) {
  switch (status) {
    case "cutting":
      return "border-[#C4A35A]/35 bg-[#C4A35A]/10 text-[#C4A35A]"
    case "scanning":
    case "exporting":
      return "border-primary/30 bg-primary/10 text-primary"
    case "paused":
      return "border-amber-500/30 bg-amber-500/10 text-amber-400"
    case "ready":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
    case "error":
      return "border-destructive/30 bg-destructive/10 text-destructive"
    default:
      return "border-border/50 bg-muted/40 text-muted-foreground"
  }
}

function StatusIcon({ status }: { status: Project["status"] }) {
  if (status === "cutting" || status === "scanning" || status === "exporting") {
    return <Loader2 className="size-3 animate-spin" />
  }
  if (status === "ready") return <CheckCircle2 className="size-3" />
  if (status === "error") return <AlertTriangle className="size-3" />
  return <Film className="size-3" />
}

export function ProjectLoader({
  projects,
  loading = false,
  activeId,
  onOpen,
  onDeleted,
  disabled,
}: ProjectLoaderProps) {
  const [query, setQuery] = useState("")
  const [selectedId, setSelectedId] = useState<string | null>(activeId)

  const sorted = useMemo(() => {
    const q = query.trim().toLowerCase()
    const filtered = projects.filter((p) => {
      if (!q) return true
      const media = mediaLabel(p.video_path, p.name).toLowerCase()
      return (
        media.includes(q) ||
        p.name.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        p.status.toLowerCase().includes(q)
      )
    })
    return filtered.sort((a, b) => {
      const ra = statusRank[a.status] ?? 9
      const rb = statusRank[b.status] ?? 9
      if (ra !== rb) return ra - rb
      return Date.parse(b.updated_at) - Date.parse(a.updated_at)
    })
  }, [projects, query])

  const selected = sorted.find((p) => p.id === selectedId) ?? null

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 280, damping: 28 }}
      className="relative flex min-h-[min(70dvh,640px)] flex-col overflow-hidden rounded-[1.35rem] border border-border/35 bg-[oklch(0.085_0.014_280)]"
    >
      <div
        aria-hidden
        className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/45 to-transparent"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(70%_50%_at_10%_0%,oklch(0.78_0.14_65/0.08),transparent_55%)]"
      />

      <div className="relative z-10 border-b border-border/35 px-6 py-6 sm:px-8">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <p className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
              Projects
            </p>
            <h2 className="font-heading mt-1.5 text-3xl font-semibold tracking-[-0.03em]">
              Load project
            </h2>
            <p className="mt-1.5 max-w-lg text-sm text-muted-foreground">
              Pick a project by name. Status and cast stay visible.
            </p>
          </div>
          <p className="timecode text-xs text-muted-foreground">
            {sorted.length} / {projects.length}
          </p>
        </div>

        <div className="relative mt-5 max-w-md">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search media, status, or id…"
            className="bg-background/40 pl-9"
            disabled={disabled}
          />
        </div>
      </div>

      <div className="relative z-10 grid min-h-0 flex-1 lg:grid-cols-[minmax(0,1.3fr)_minmax(280px,0.7fr)]">
        <ScrollArea className="min-h-[320px] max-h-[min(58dvh,560px)] border-b border-border/35 lg:border-b-0 lg:border-r lg:border-border/35">
          {loading ? (
            <div className="px-5 py-8">
              <LoadingBlock label="Loading projects…" rows={5} dense />
            </div>
          ) : sorted.length === 0 ? (
            <div className="flex flex-col items-center justify-center gap-3 px-6 py-20 text-center">
              <FolderOpen className="size-9 text-muted-foreground/35" />
              <p className="text-sm text-muted-foreground">
                {projects.length === 0
                  ? "No projects yet. Upload footage first."
                  : "No matches for that search."}
              </p>
            </div>
          ) : (
            <div className="divide-y divide-border/30">
              {sorted.map((project, index) => {
                const media = mediaLabel(project.video_path, project.name)
                const active = project.id === selectedId
                const pct = Math.round((project.progress || 0) * 100)
                return (
                  <motion.button
                    key={project.id}
                    type="button"
                    disabled={disabled}
                    initial={{ opacity: 0, y: 6 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{
                      type: "spring",
                      stiffness: 320,
                      damping: 28,
                      delay: Math.min(index * 0.025, 0.2),
                    }}
                    onClick={() => setSelectedId(project.id)}
                    className={cn(
                      "w-full px-5 py-3.5 text-left transition",
                      active
                        ? "bg-primary/[0.06]"
                        : "hover:bg-background/30",
                    )}
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          {active ? (
                            <span className="size-1.5 shrink-0 rounded-full bg-primary" />
                          ) : null}
                          <p className="truncate font-medium tracking-tight">{media}</p>
                        </div>
                        <p
                          className="mt-0.5 truncate font-mono text-[10px] text-muted-foreground"
                          title={project.video_path}
                        >
                          {project.video_path}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-start gap-1.5">
                        <span
                          className={cn(
                            "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-[10px] font-medium capitalize",
                            statusTone(project.status),
                          )}
                        >
                          <StatusIcon status={project.status} />
                          {friendlyProjectStatus(project.status)}
                        </span>
                        {onDeleted ? (
                          <DeleteProjectButton
                            project={project}
                            onDeleted={async (id) => {
                              if (selectedId === id) setSelectedId(null)
                              await onDeleted(id)
                            }}
                          />
                        ) : null}
                      </div>
                    </div>

                    {(project.status === "cutting" ||
                      project.status === "scanning" ||
                      project.status === "exporting") && (
                      <div className="mt-3 space-y-1">
                        <div className="flex justify-between text-[10px] text-muted-foreground">
                          <span className="truncate">{sanitizeStage(project.stage) || "Working…"}</span>
                          <span className="timecode tabular-nums">{pct}%</span>
                        </div>
                        <Progress value={pct} className="h-1 bg-muted/70" />
                      </div>
                    )}

                    <div className="mt-2.5 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                      <span className="inline-flex items-center gap-1">
                        <Clock3 className="size-3" />
                        {formatRelativeTime(project.updated_at)}
                      </span>
                      <span className="inline-flex items-center gap-1">
                        <ScanFace className="size-3" />
                        {project.people.length} cast
                      </span>
                      {project.duration_sec > 0 ? (
                        <span className="timecode">{formatDuration(project.duration_sec)}</span>
                      ) : null}
                    </div>
                  </motion.button>
                )
              })}
            </div>
          )}
        </ScrollArea>

        <aside className="flex flex-col justify-between gap-4 p-6">
          {selected ? (
            <>
              <div className="space-y-4">
                <div>
                  <p className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                    Selected
                  </p>
                  <h3 className="font-heading mt-1 text-xl leading-tight tracking-tight">
                    {mediaLabel(selected.video_path, selected.name)}
                  </h3>
                  <p className="timecode mt-1 text-xs text-muted-foreground">{selected.id}</p>
                </div>

                <dl className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border/35 bg-border/35">
                  <Meta label="State" value={friendlyProjectStatus(selected.status)} />
                  <Meta
                    label="Progress"
                    value={`${Math.round((selected.progress || 0) * 100)}%`}
                  />
                  <Meta label="Created" value={formatAbsoluteStamp(selected.created_at)} />
                  <Meta label="Last used" value={formatAbsoluteStamp(selected.updated_at)} />
                  <Meta label="Footage type" value={friendlyBackend(selected.backend)} />
                  <Meta label="Quality pack" value={friendlyModel(selected.backend, selected.model)} />
                  <Meta label="Cast" value={String(selected.people.length)} />
                  <Meta label="Scenes" value={String(selected.segments.length)} />
                </dl>

                {selected.error ? (
                  <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    {friendlyError(selected.error, "Scan failed. Try again.")}
                  </p>
                ) : null}
              </div>

              <div className="flex flex-col gap-2">
                <AnimatedButton
                  className="h-10 w-full"
                  disabled={disabled}
                  onClick={() => onOpen(selected)}
                >
                  {selected.status === "ready"
                    ? "Open in gallery"
                    : selected.status === "cutting" ||
                        selected.status === "scanning" ||
                        selected.status === "paused"
                      ? "Open scan"
                      : "Load project"}
                </AnimatedButton>
                {onDeleted ? (
                  <DeleteProjectButton
                    project={selected}
                    showLabel
                    className="h-9 w-full justify-center"
                    onDeleted={async (id) => {
                      if (selectedId === id) setSelectedId(null)
                      await onDeleted(id)
                    }}
                  />
                ) : null}
              </div>
            </>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center text-sm text-muted-foreground">
              <Film className="size-8 opacity-35" />
              Select a project.
            </div>
          )}
        </aside>
      </div>
    </motion.div>
  )
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-[oklch(0.09_0.014_280)] px-3 py-2.5">
      <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 truncate text-sm font-medium capitalize">{value}</dd>
    </div>
  )
}
