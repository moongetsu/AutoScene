import { Aperture } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { AnimeCVModelInfo } from "@/lib/api"
import {
  familyHasLevels,
  findFamilyByModelId,
  INSIGHTFACE_FAMILIES,
  properCaseWireLabel,
} from "@/lib/modelFamilies"
import { friendlyModel } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type ScanProfileRackProps = {
  backend: string
  model: string
  animeModels: AnimeCVModelInfo[]
  gpuLabel?: string
  disabled?: boolean
  /** slim = monitor-wall strip; full = idle camera head */
  variant?: "full" | "slim"
  onBackend: (backend: string) => void
  onModel: (model: string) => void
}

function InsightFaceModelControls({
  model,
  disabled,
  slim,
  onModel,
}: {
  model: string
  disabled?: boolean
  slim: boolean
  onModel: (model: string) => void
}) {
  const family =
    findFamilyByModelId(INSIGHTFACE_FAMILIES, model) ?? INSIGHTFACE_FAMILIES[0]
  const showLevel = familyHasLevels(family)
  const levelId = family.levels.some((l) => l.id === model)
    ? model
    : family.defaultLevelId

  return (
    <div className={cn("space-y-1.5", slim && "sm:col-span-2 space-y-2")}>
      {!slim ? (
        <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
          Quality pack
        </Label>
      ) : null}
      <div className={cn(slim ? "grid gap-2 sm:grid-cols-2" : "space-y-3")}>
        <Select
          value={family.id}
          onValueChange={(v) => {
            if (!v) return
            const next = INSIGHTFACE_FAMILIES.find((f) => f.id === v)
            if (!next) return
            onModel(next.defaultLevelId)
          }}
          disabled={disabled}
        >
          <SelectTrigger className={cn("bg-background/40", slim && "h-9")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {INSIGHTFACE_FAMILIES.map((f) => (
              <SelectItem key={f.id} value={f.id}>
                {f.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        {showLevel ? (
          <div className="space-y-1.5">
            {!slim ? (
              <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                Level
              </Label>
            ) : null}
            <Select
              value={levelId}
              onValueChange={(v) => v && onModel(v)}
              disabled={disabled}
            >
              <SelectTrigger className={cn("bg-background/40", slim && "h-9")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {family.levels.map((l) => (
                  <SelectItem key={l.id} value={l.id}>
                    {l.recommended ? `${l.label} (recommended)` : l.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}
      </div>
    </div>
  )
}

export function ScanProfileRack({
  backend,
  model,
  animeModels,
  gpuLabel,
  disabled,
  variant = "full",
  onBackend,
  onModel,
}: ScanProfileRackProps) {
  const slim = variant === "slim"

  const slimStatus = friendlyModel(backend, model)

  return (
    <div
      className={cn(
        "relative overflow-hidden",
        slim
          ? "rounded-xl border border-border/35 bg-[oklch(0.09_0.014_280/0.9)] px-3.5 py-3"
          : "rounded-[1.2rem] border border-border/35 bg-[oklch(0.09_0.014_280)] p-5",
      )}
    >
      {/* Amber hairline — camera head cue */}
      <div
        aria-hidden
        className={cn(
          "absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/55 to-transparent",
          slim && "via-primary/40",
        )}
      />

      <div className={cn("relative", slim ? "space-y-3" : "space-y-5")}>
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2.5">
            <span
              className={cn(
                "flex items-center justify-center rounded-md border border-primary/25 bg-primary/10 text-primary",
                slim ? "size-7" : "size-8",
              )}
            >
              <Aperture className={slim ? "size-3.5" : "size-4"} strokeWidth={1.6} />
            </span>
            <div>
              <p className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                Scan
              </p>
              <h2
                className={cn(
                  "font-heading tracking-tight",
                  slim ? "text-sm" : "text-base",
                )}
              >
                {slim ? "Quality" : "Scan profile"}
              </h2>
            </div>
          </div>
          {slim ? (
            <Badge variant="secondary" className="timecode text-[10px]">
              {slimStatus}
            </Badge>
          ) : null}
        </div>

        <div className={cn(slim ? "grid gap-2 sm:grid-cols-2" : "space-y-4")}>
          <div className="space-y-1.5">
            {!slim ? (
              <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                Footage type
              </Label>
            ) : null}
            <Select
              value={backend}
              onValueChange={(v) => {
                if (!v) return
                onBackend(v)
              }}
              disabled={disabled}
            >
              <SelectTrigger className={cn("bg-background/40", slim && "h-9")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="insightface">Live action / 3D</SelectItem>
                <SelectItem value="animecv">Anime / cartoon</SelectItem>
                <SelectItem value="deepface">Lab compare (A/B only)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {backend === "animecv" ? (
            <div className={cn("space-y-2", slim && "sm:col-span-2")}>
              {!slim ? (
                <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                  Quality pack
                </Label>
              ) : null}
              <div className={cn("grid gap-2", slim ? "sm:grid-cols-2" : "")}>
                {animeModels.map((m) => {
                  const active = model === m.id
                  const quality = m.role === "quality"
                  return (
                    <button
                      key={m.id}
                      type="button"
                      disabled={disabled}
                      onClick={() => onModel(m.id)}
                      className={cn(
                        "relative overflow-hidden rounded-lg border text-left transition",
                        slim ? "px-2.5 py-2" : "px-3 py-3",
                        active
                          ? "border-primary/45 bg-primary/[0.06]"
                          : "border-border/35 bg-background/25 hover:border-border/60 hover:bg-background/40",
                      )}
                    >
                      <div className="relative flex items-start justify-between gap-2">
                        <div>
                          <p className="font-heading text-sm tracking-tight">{m.label}</p>
                          {!slim ? (
                            <p className="mt-0.5 text-[11px] italic text-primary/90">
                              {m.tagline}
                            </p>
                          ) : null}
                        </div>
                        <Badge
                          variant="secondary"
                          className={cn(
                            "timecode shrink-0 text-[10px]",
                            active && "border-primary/30 text-primary",
                          )}
                        >
                          {quality ? "Canon" : "Flash"}
                        </Badge>
                      </div>
                      {!slim ? (
                        <p className="relative mt-2 text-[11px] leading-relaxed text-muted-foreground">
                          {m.blurb}
                        </p>
                      ) : null}
                    </button>
                  )
                })}
              </div>
              {!slim ? (
                <p className="text-[11px] text-muted-foreground">
                  Missing models? Settings → Models → Install all.
                </p>
              ) : null}
            </div>
          ) : backend === "insightface" ? (
            <InsightFaceModelControls
              model={model}
              disabled={disabled}
              slim={slim}
              onModel={onModel}
            />
          ) : (
            <div className="space-y-1.5">
              {!slim ? (
                <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                  Quality pack
                </Label>
              ) : null}
              <Select
                value={model}
                onValueChange={(v) => v && onModel(v)}
                disabled={disabled}
              >
                <SelectTrigger className={cn("bg-background/40", slim && "h-9")}>
                  <SelectValue placeholder={properCaseWireLabel(model)} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="Facenet512">Standard</SelectItem>
                  <SelectItem value="ArcFace">Sharp</SelectItem>
                  <SelectItem value="Buffalo_L">Large</SelectItem>
                </SelectContent>
              </Select>
            </div>
          )}
        </div>

        {gpuLabel && !slim ? (
          <p className="border-t border-border/30 pt-3 text-[11px] text-muted-foreground">
            {gpuLabel}
          </p>
        ) : null}
      </div>
    </div>
  )
}
