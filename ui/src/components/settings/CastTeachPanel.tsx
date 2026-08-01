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
      className={cn("panel-surface overflow-hidden", className)}
    >
      <header className="border-b border-border/40 px-5 py-4">
        <h2 className="font-heading text-base tracking-tight">Teach names</h2>
        <p className="mt-1 max-w-[64ch] text-xs leading-relaxed text-muted-foreground">
          Give AutoScene a photo of a character and it will label them across your scanned
          footage. Optional — you can also rename characters by hand in Gallery.
        </p>
      </header>
      <div className="grid gap-0 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
        <div className="border-b border-border/40 p-5 lg:border-b-0 lg:border-r">
          <p className="max-w-[42ch] text-sm leading-relaxed text-muted-foreground">
            Name each image file after the character (for example{" "}
            <span className="font-mono text-xs">Ahsoka.png</span>), or pin a photo to a character
            from the Gallery dossier.
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

        <div className="flex flex-col justify-center gap-3 p-5">
          <Button
            type="button"
            className="min-h-11 gap-2"
            disabled={!ready}
            onClick={() => setPackOpen(true)}
          >
            <ImagePlus className="size-4" />
            Add character photos
          </Button>
          <Button
            type="button"
            variant="outline"
            className="min-h-11 gap-2"
            disabled={!ready}
            onClick={() => nav("/gallery")}
          >
            <ScanFace className="size-4" />
            Open Gallery
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
              Available once a project has finished scanning.
            </p>
          ) : null}
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
