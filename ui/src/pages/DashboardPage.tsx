import { Link, useNavigate } from "react-router-dom"
import { ArrowRight, Film, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent } from "@/components/ui/card"
import { PageHeader } from "@/components/app/PageHeader"
import { DeleteProjectButton } from "@/components/dashboard/DeleteProjectButton"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { useProjects } from "@/state/project"
import { formatDuration } from "@/lib/api"
import { friendlyGpuLine, friendlyProjectStatus } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

const statusTone: Record<string, string> = {
  ready: "border-primary/30 bg-primary/5",
  scanning: "border-amber-500/30 bg-amber-500/5",
  error: "border-destructive/30 bg-destructive/5",
  idle: "border-border/50 bg-card/40",
}

export function DashboardPage() {
  const nav = useNavigate()
  const {
    projects,
    projectsReady,
    activeId,
    setActiveId,
    refreshProjects,
    removeProjectLocal,
    resetImportUi,
    gpu,
    gpuReady,
  } = useProjects()

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto">
      <PageHeader
        eyebrow="Workspace"
        title="Dashboard"
        description="Import video. Find characters. Export scene packs."
        actions={
          <Button asChild>
            <Link to="/import">
              New project
              <ArrowRight className="size-4" />
            </Link>
          </Button>
        }
      />

      <div className="flex flex-1 flex-col gap-8 p-6 lg:p-8 animate-fade-up">
        <Card className="panel-surface overflow-hidden border-primary/20 bg-gradient-to-br from-card/90 to-card/40">
          <CardContent className="relative p-8">
            <div className="pointer-events-none absolute -right-24 -top-24 size-72 rounded-full bg-primary/10 blur-3xl" />
            <div className="relative max-w-xl">
              <Badge variant="secondary" className="mb-4 gap-1.5 border-primary/20">
                <Sparkles className="size-3.5 text-primary" />
                On-device
              </Badge>
              <h2 className="font-heading text-3xl leading-tight tracking-tight">
                Import video. Find characters. Export scene packs.
              </h2>
              <p className="mt-3 max-w-lg text-sm leading-relaxed text-muted-foreground">
                Your footage stays on this machine.
              </p>
              <div className="mt-6 flex flex-wrap items-center gap-3">
                <Button asChild size="lg" className="h-9 px-4 text-sm">
                  <Link to="/import">Start import</Link>
                </Button>
                {gpuReady ? (
                  gpu ? (
                    <span className="text-xs text-muted-foreground">
                      {friendlyGpuLine(gpu.available, gpu.name ?? gpu.provider, gpu.detail)}
                    </span>
                  ) : null
                ) : (
                  <span className="text-xs text-muted-foreground">Checking graphics…</span>
                )}
              </div>
            </div>
          </CardContent>
        </Card>

        <section>
          <div className="mb-4 flex items-end justify-between">
            <h3 className="font-heading text-lg">Recent projects</h3>
            <span className="text-xs text-muted-foreground">
              {projectsReady ? `${projects.length} total` : "Loading…"}
            </span>
          </div>

          {!projectsReady ? (
            <div className="panel-surface px-5 py-6">
              <LoadingBlock label="Loading projects…" rows={4} dense />
            </div>
          ) : projects.length === 0 ? (
            <div className="panel-surface flex flex-col items-center gap-4 px-6 py-16 text-center">
              <Film className="size-8 text-muted-foreground/50" />
              <p className="text-sm text-muted-foreground">No projects yet. Drop footage to begin.</p>
              <Button variant="outline" onClick={() => nav("/import")}>
                Import video
              </Button>
            </div>
          ) : (
            <div className="stagger grid gap-3">
              {projects.map((p) => (
                <div
                  key={p.id}
                  className={cn(
                    "panel-surface flex w-full items-center gap-3 px-5 py-4 transition hover:border-primary/35",
                    statusTone[p.status] ?? statusTone.idle,
                  )}
                >
                  <button
                    type="button"
                    onClick={() => {
                      setActiveId(p.id)
                      nav(p.status === "ready" ? "/gallery" : "/import")
                    }}
                    className="min-w-0 flex-1 text-left"
                  >
                    <p className="truncate font-medium">{p.name}</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {friendlyProjectStatus(p.status)}
                      {p.duration_sec > 0 ? ` · ${formatDuration(p.duration_sec)}` : ""}
                      {p.people?.length ? ` · ${p.people.length} characters` : ""}
                    </p>
                  </button>
                  <span className="shrink-0 text-xs font-medium text-primary">Open</span>
                  <DeleteProjectButton
                    project={p}
                    showLabel
                    onDeleted={async (id) => {
                      removeProjectLocal(id)
                      if (activeId === id) {
                        setActiveId(null)
                        resetImportUi()
                      }
                      await refreshProjects()
                    }}
                  />
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  )
}
