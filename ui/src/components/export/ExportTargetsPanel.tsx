import { Clapperboard, FileVideo } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { FacePortrait } from "@/components/shared/FacePortrait"
import { api, formatDuration, type MergedExport, type Person } from "@/lib/api"
import { cn } from "@/lib/utils"

export type ExportCastPack = {
  person: Person
  sceneCount: number
  activeDuration: number
}

type ExportTargetsPanelProps = {
  projectId: string
  packs: ExportCastPack[]
  files: MergedExport[]
  onSelectPack: (personId: number) => void
  onSelectFile: (file: MergedExport) => void
}

export function ExportTargetsPanel({
  projectId,
  packs,
  files,
  onSelectPack,
  onSelectFile,
}: ExportTargetsPanelProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="border-b border-border/40 px-5 py-5 lg:px-6">
        <p className="text-[10px] uppercase tracking-[0.2em] text-muted-foreground">
          Export bay
        </p>
        <h1 className="mt-1 text-2xl font-semibold tracking-tight">Exports</h1>
        <p className="mt-1 max-w-xl text-sm text-muted-foreground">
          Pick a cast pack to review scenes, or open a finished merge.
        </p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="space-y-8 p-5 lg:p-6">
          <section className="space-y-3">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-[10px] uppercase tracking-[0.2em] text-muted-foreground">
                Cast packs
              </p>
              <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
                {packs.length}
              </span>
            </div>
            {packs.length === 0 ? (
              <p className="text-sm text-muted-foreground">No cast with scenes yet.</p>
            ) : (
              <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
                {packs.map(({ person, sceneCount, activeDuration }) => (
                  <button
                    key={person.person_id}
                    type="button"
                    onClick={() => onSelectPack(person.person_id)}
                    className={cn(
                      "group flex items-center gap-3 rounded-lg border border-border/45 bg-card/30 px-3 py-2.5 text-left transition",
                      "hover:border-primary/40 hover:bg-primary/5",
                      "active:translate-y-px",
                    )}
                  >
                    <FacePortrait
                      src={
                        person.thumbnail_path
                          ? api.mediaUrl(projectId, person.thumbnail_path)
                          : null
                      }
                      size="sm"
                      fit="cover"
                      className="size-12 rounded-md border-border/40"
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium tracking-tight">
                        {person.label}
                      </p>
                      <p className="mt-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
                        {sceneCount} scene{sceneCount === 1 ? "" : "s"}
                        {activeDuration > 0 ? ` · ${formatDuration(activeDuration)}` : ""}
                      </p>
                    </div>
                    <Clapperboard className="size-4 shrink-0 text-muted-foreground/70 transition group-hover:text-primary" />
                  </button>
                ))}
              </div>
            )}
          </section>

          <section className="space-y-3">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-[10px] uppercase tracking-[0.2em] text-muted-foreground">
                Finished files
              </p>
              <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
                {files.length}
              </span>
            </div>
            {files.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                Merged videos land here after export completes.
              </p>
            ) : (
              <ul className="divide-y divide-border/40 rounded-lg border border-border/45">
                {files.map((file) => (
                  <li key={`${file.id}-${file.output_path}`}>
                    <button
                      type="button"
                      onClick={() => onSelectFile(file)}
                      className={cn(
                        "flex w-full items-start gap-3 px-3 py-3 text-left transition",
                        "hover:bg-primary/5 active:translate-y-px",
                      )}
                    >
                      <div className="mt-0.5 rounded-md bg-primary/10 p-2 text-primary">
                        <FileVideo className="size-4" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <p className="truncate text-sm font-medium">{file.person_label}</p>
                          <Badge variant="secondary" className="tabular-nums">
                            {file.scene_count} scenes
                          </Badge>
                        </div>
                        <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                          {file.output_path}
                        </p>
                      </div>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  )
}
