import { useEffect, useState } from "react"
import { useNavigate } from "react-router-dom"
import { ImagePlus, ScanFace, Sparkles } from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { CastPackDialog } from "@/components/gallery/CastPackDialog"
import { api } from "@/lib/api"
import { friendlyError } from "@/lib/plainCopy"
import { useProjects } from "@/state/project"
import { cn } from "@/lib/utils"

type CastTeachPanelProps = {
  className?: string
}

export function CastTeachPanel({ className }: CastTeachPanelProps) {
  const nav = useNavigate()
  const { active, refreshActive } = useProjects()
  const [packOpen, setPackOpen] = useState(false)
  const [animeReady, setAnimeReady] = useState<boolean | null>(null)
  const [installing, setInstalling] = useState(false)
  const ready = active?.status === "ready" && (active.people?.length ?? 0) > 0

  useEffect(() => {
    let alive = true
    void api
      .modelsStatus()
      .then((v) => {
        if (!alive) return
        const anime = v.animecv_models ?? []
        const animeOk = anime.length > 0 && anime.every((m) => m.installed)
        const animecvPkg = (v.deps ?? []).some(
          (d) => d.id === "animecv_pkg" && d.installed,
        )
        setAnimeReady(Boolean(animeOk && animecvPkg))
      })
      .catch(() => {
        if (alive) setAnimeReady(null)
      })
    return () => {
      alive = false
    }
  }, [])

  const downloadAnime = async () => {
    setInstalling(true)
    try {
      await api.modelsInstall([
        "torch_cuda",
        "animecv_pkg",
        "animecv_detector",
        "cel_canon",
        "ink_flash",
      ])
      toast.message("Anime pack download started. Watch Model vault progress.")
      const v = await api.modelsStatus()
      const anime = v.animecv_models ?? []
      setAnimeReady(anime.length > 0 && anime.every((m) => m.installed))
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setInstalling(false)
    }
  }

  return (
    <div
      className={cn(
        "overflow-hidden rounded-2xl border border-border/50 bg-card/40",
        className,
      )}
    >
      <div className="grid gap-0 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
        <div className="border-b border-border/40 p-5 lg:border-b-0 lg:border-r lg:p-6">
          <p className="text-[11px] uppercase tracking-[0.22em] text-muted-foreground">
            Cast library
          </p>
          <h2 className="font-heading mt-2 text-2xl tracking-tight">Teach faces</h2>
          <p className="mt-2 max-w-[36ch] text-sm leading-relaxed text-muted-foreground">
            Drop named stills (Ahsoka.png) or pin a photo on a character in Gallery. Updates the
            character match and scene plates from prior scan data.
          </p>
          <div className="mt-4 flex flex-wrap gap-2">
            {animeReady === null ? (
              <Badge variant="secondary" className="text-muted-foreground">
                Checking models…
              </Badge>
            ) : animeReady === false ? (
              <Badge variant="secondary" className="border-amber-500/30 text-amber-400">
                Anime models incomplete
              </Badge>
            ) : (
              <Badge variant="secondary" className="border-primary/30 text-primary">
                Models ready
              </Badge>
            )}
            {ready ? (
              <Badge variant="secondary">{active!.people.length} cast in project</Badge>
            ) : (
              <Badge variant="secondary">Scan a project first</Badge>
            )}
          </div>
        </div>

        <div className="flex flex-col justify-center gap-3 p-5 lg:p-6">
          <Button
            type="button"
            className="min-h-11 gap-2"
            disabled={!ready}
            onClick={() => setPackOpen(true)}
          >
            <ImagePlus className="size-4" />
            Cast photos
          </Button>
          <Button
            type="button"
            variant="outline"
            className="min-h-11 gap-2"
            disabled={!ready}
            onClick={() => nav("/gallery")}
          >
            <ScanFace className="size-4" />
            Open gallery dossier
          </Button>
          {animeReady === false ? (
            <Button
              type="button"
              variant="outline"
              className="min-h-11 gap-2"
              disabled={installing}
              onClick={() => void downloadAnime()}
            >
              <Sparkles className="size-4 text-primary" />
              {installing ? "Starting download…" : "Download anime packs"}
            </Button>
          ) : null}
          {!ready ? (
            <p className="text-xs text-muted-foreground">
              Import footage, finish a scan, then teach names here or in Gallery.
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              Filename = character name in pack mode. Dossier Teach face pins to the selected person.
            </p>
          )}
        </div>
      </div>

      {active ? (
        <CastPackDialog
          open={packOpen}
          onOpenChange={setPackOpen}
          projectId={active.id}
          onDone={async () => {
            await refreshActive()
            toast.success("Cast updated")
          }}
        />
      ) : null}
    </div>
  )
}
