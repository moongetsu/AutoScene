import { useEffect, useState } from "react"
import { motion } from "motion/react"
import {
  Cpu,
  Gauge,
  HardDrive,
  RefreshCw,
  Save,
  ScanFace,
  SlidersHorizontal,
} from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { Slider } from "@/components/ui/slider"
import { PageHeader } from "@/components/app/PageHeader"
import { CastTeachPanel } from "@/components/settings/CastTeachPanel"
import { ModelVault } from "@/components/settings/ModelVault"
import { CopyButton } from "@/components/shared/CopyButton"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import {
  CONTENT_PRESET_FALLBACK,
  type AppSettings,
  type ContentPresetId,
  type FaceSelectMode,
} from "@/lib/api"
import { useProjects } from "@/state/project"
import { useAppSettings } from "@/state/settings"
import { friendlyBackend, friendlyGpuLine, friendlyModel, friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

const FACE_MODES: {
  id: FaceSelectMode
  title: string
  blurb: string
}[] = [
  {
    id: "all",
    title: "All faces",
    blurb: "Every detection above min size. Max cast coverage.",
  },
  {
    id: "largest",
    title: "Largest only",
    blurb: "One face per frame. Lead subject focus.",
  },
  {
    id: "relative",
    title: "Relative cut",
    blurb: "Keep faces at least this % of the largest in-frame.",
  },
  {
    id: "top_n",
    title: "Top N",
    blurb: "N largest faces. Two-shots and small ensembles.",
  },
]

const PRESET_ACCENT: Record<string, string> = {
  live_action: "from-primary/15 via-transparent to-transparent",
  animation: "from-[oklch(0.55_0.08_250/0.18)] via-transparent to-transparent",
  anime: "from-[oklch(0.65_0.12_25/0.2)] via-transparent to-transparent",
}

export function SettingsPage() {
  const { gpu, gpuReady, refreshGpu } = useProjects()
  const {
    settings,
    settingsError,
    refreshSettings,
    saveSettings,
  } = useAppSettings()
  const [draft, setDraft] = useState<AppSettings | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (settings) setDraft(settings)
  }, [settings])

  useEffect(() => {
    void refreshSettings()
  }, [refreshSettings])

  const dirty =
    !!settings &&
    !!draft &&
    (settings.sample_fps !== draft.sample_fps ||
      settings.det_size !== draft.det_size ||
      settings.detect_max_edge !== draft.detect_max_edge ||
      settings.prefetch_depth !== draft.prefetch_depth ||
      settings.min_face_size !== draft.min_face_size ||
      settings.cluster_min_size !== draft.cluster_min_size ||
      settings.cluster_merge_sim !== draft.cluster_merge_sim ||
      settings.content_preset !== draft.content_preset ||
      settings.default_backend !== draft.default_backend ||
      settings.default_model !== draft.default_model ||
      settings.face_select_mode !== draft.face_select_mode ||
      settings.face_relative_pct !== draft.face_relative_pct ||
      settings.face_top_n !== draft.face_top_n ||
      settings.scene_cut_detector !== draft.scene_cut_detector ||
      settings.scene_cut_adaptive_threshold !== draft.scene_cut_adaptive_threshold ||
      settings.scene_cut_transnet_threshold !== draft.scene_cut_transnet_threshold)

  const save = async () => {
    if (!draft) return
    setSaving(true)
    try {
      const next = await saveSettings({
        sample_fps: draft.sample_fps,
        det_size: draft.det_size,
        detect_max_edge: draft.detect_max_edge,
        prefetch_depth: draft.prefetch_depth,
        min_face_size: draft.min_face_size,
        cluster_min_size: draft.cluster_min_size,
        cluster_merge_sim: draft.cluster_merge_sim,
        content_preset: draft.content_preset,
        default_backend: draft.default_backend,
        default_model: draft.default_model,
        face_select_mode: draft.face_select_mode,
        face_relative_pct: draft.face_relative_pct,
        face_top_n: draft.face_top_n,
        scene_cut_detector: draft.scene_cut_detector,
        scene_cut_adaptive_threshold: draft.scene_cut_adaptive_threshold,
        scene_cut_transnet_threshold: draft.scene_cut_transnet_threshold,
      })
      setDraft(next)
      toast.success("Settings saved. New scans use these from the next start.")
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto">
      <PageHeader
        eyebrow="Settings"
        title="Settings"
        description="Install models, pick a content preset, and tune scan options. Changes apply on the next scan."
        actions={
          <Button size="sm" disabled={!dirty || saving || !draft} onClick={() => void save()}>
            <Save className="size-3.5" />
            {saving ? "Saving…" : dirty ? "Save" : "Saved"}
          </Button>
        }
      />

      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-5 lg:gap-8 lg:p-8">
        {/* Signal + Armory */}
        <div className="grid gap-4 lg:grid-cols-12 lg:gap-5">
          <motion.section
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ type: "spring", stiffness: 260, damping: 28 }}
            className="panel-surface relative overflow-hidden lg:col-span-4"
          >
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0 bg-gradient-to-br from-primary/10 via-transparent to-transparent"
            />
            <div className="relative flex h-full flex-col justify-between p-5">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
                    Device
                  </p>
                  <h2 className="font-heading mt-1 flex items-center gap-2 text-xl tracking-tight">
                    <Cpu className="size-4 text-primary" />
                    Graphics
                  </h2>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={refreshing}
                  className="min-h-9"
                  onClick={async () => {
                    setRefreshing(true)
                    try {
                      await refreshGpu()
                    } finally {
                      setRefreshing(false)
                    }
                  }}
                >
                  <RefreshCw className={`size-3.5 ${refreshing ? "animate-spin" : ""}`} />
                  Refresh
                </Button>
              </div>
              {gpuReady && gpu ? (
                <div className="mt-6 space-y-3">
                  <Badge variant={gpu.available ? "default" : "secondary"}>
                    {gpu.available ? "Graphics ready" : "Graphics unavailable"}
                  </Badge>
                  <p className="text-sm font-medium leading-snug">
                    {friendlyGpuLine(gpu.available, gpu.name, gpu.detail)}
                  </p>
                </div>
              ) : (
                <div className="mt-6">
                  <LoadingBlock label="Loading graphics status…" rows={3} />
                </div>
              )}
            </div>
          </motion.section>

          <motion.div
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.05, type: "spring", stiffness: 260, damping: 28 }}
            className="lg:col-span-8"
          >
            <ModelVault />
          </motion.div>
        </div>

        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.08, type: "spring", stiffness: 260, damping: 28 }}
        >
          <CastTeachPanel />
        </motion.div>

        {/* Footage language */}
        <section className="space-y-3">
          <div className="flex items-end justify-between gap-3 px-0.5">
            <div>
              <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
                Content
              </p>
              <h2 className="font-heading mt-1 text-xl tracking-tight">Content preset</h2>
            </div>
            {!draft?.content_preset ? (
              <Badge variant="secondary" className="text-amber-400/90">
                Custom
              </Badge>
            ) : null}
          </div>
          {draft ? (
            <div className="grid gap-3 md:grid-cols-3">
              {(draft.content_presets?.length
                ? draft.content_presets
                : CONTENT_PRESET_FALLBACK
              ).map((preset, i) => {
                const active = draft.content_preset === preset.id
                return (
                  <motion.button
                    key={preset.id}
                    type="button"
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ delay: 0.04 * i }}
                    onClick={() => {
                      const id = preset.id as Exclude<ContentPresetId, "">
                      setDraft({
                        ...draft,
                        content_preset: id,
                        cluster_merge_sim: preset.cluster_merge_sim,
                        cluster_min_size: preset.cluster_min_size,
                        min_face_size: preset.min_face_size,
                        det_size: preset.det_size,
                        default_backend: preset.default_backend || draft.default_backend,
                        default_model: preset.default_model || draft.default_model,
                      })
                    }}
                    className={cn(
                      "relative overflow-hidden rounded-2xl border px-4 py-4 text-left transition",
                      active
                        ? "border-primary/50 bg-card/80 shadow-[0_0_0_1px_oklch(0.78_0.14_65/0.25)]"
                        : "border-border/40 bg-card/40 hover:border-border hover:bg-card/60",
                      i === 1 && "md:translate-y-2",
                    )}
                  >
                    <div
                      aria-hidden
                      className={cn(
                        "pointer-events-none absolute inset-0 bg-gradient-to-br",
                        PRESET_ACCENT[preset.id] || PRESET_ACCENT.live_action,
                      )}
                    />
                    <div className="relative">
                      <div className="flex items-center justify-between gap-2">
                        <p className="font-heading text-base tracking-tight">{preset.label}</p>
                        {active ? (
                          <Badge variant="secondary" className="border-primary/30 text-primary">
                            Active
                          </Badge>
                        ) : null}
                      </div>
                      <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                        {preset.blurb}
                      </p>
                      <p className="timecode mt-3 text-[11px] tabular-nums text-muted-foreground">
                        {friendlyBackend(preset.default_backend)} ·{" "}
                        {friendlyModel(preset.default_backend, preset.default_model)}
                      </p>
                    </div>
                  </motion.button>
                )
              })}
            </div>
          ) : (
            <div className="panel-surface px-5 py-5">
              <LoadingBlock
                label={
                  settingsError
                    ? "Could not load presets — is AutoScene running?"
                    : "Loading presets…"
                }
                rows={3}
                dense
              />
            </div>
          )}
        </section>

        {/* Face + Throughput deck */}
        <div className="grid gap-5 lg:grid-cols-12">
          <section className="panel-surface p-5 lg:col-span-5">
            <div className="mb-4 flex items-center gap-2">
              <ScanFace className="size-4 text-primary" />
              <h2 className="font-heading text-lg tracking-tight">Face detection</h2>
            </div>
            {draft ? (
              <div className="space-y-3">
                <div className="grid grid-cols-2 gap-2">
                  {FACE_MODES.map((mode) => {
                    const active = draft.face_select_mode === mode.id
                    return (
                      <button
                        key={mode.id}
                        type="button"
                        onClick={() => setDraft({ ...draft, face_select_mode: mode.id })}
                        className={cn(
                          "min-h-[5.5rem] rounded-xl border px-3 py-3 text-left transition",
                          active
                            ? "border-primary/45 bg-primary/5"
                            : "border-border/40 bg-background/30 hover:border-border",
                        )}
                      >
                        <p className="text-sm font-medium">{mode.title}</p>
                        <p className="mt-1 text-[11px] leading-snug text-muted-foreground">
                          {mode.blurb}
                        </p>
                      </button>
                    )
                  })}
                </div>
                {draft.face_select_mode === "relative" ? (
                  <div className="space-y-3 border-t border-border/35 pt-4">
                    <div className="flex items-end justify-between">
                      <Label htmlFor="relative-pct">Vs largest</Label>
                      <span className="timecode text-sm text-primary">
                        {Math.round(draft.face_relative_pct)}%
                      </span>
                    </div>
                    <Slider
                      id="relative-pct"
                      min={1}
                      max={100}
                      step={1}
                      value={[Math.round(draft.face_relative_pct)]}
                      onValueChange={(v) => {
                        const n = Array.isArray(v) ? v[0] : v
                        if (typeof n === "number") {
                          setDraft({ ...draft, face_relative_pct: n })
                        }
                      }}
                    />
                  </div>
                ) : null}
                {draft.face_select_mode === "top_n" ? (
                  <div className="space-y-2 border-t border-border/35 pt-4">
                    <Label htmlFor="top-n">Keep N</Label>
                    <Input
                      id="top-n"
                      type="number"
                      min={1}
                      max={12}
                      className="max-w-[8rem] bg-background/50"
                      value={draft.face_top_n}
                      onChange={(e) =>
                        setDraft({
                          ...draft,
                          face_top_n: Math.max(1, Math.min(12, Number(e.target.value) || 1)),
                        })
                      }
                    />
                  </div>
                ) : null}
              </div>
            ) : (
              <LoadingBlock label="Loading face options…" rows={4} dense />
            )}
          </section>

          <section className="panel-surface p-5 lg:col-span-7">
            <div className="mb-4 flex items-center gap-2">
              <Gauge className="size-4 text-primary" />
              <h2 className="font-heading text-lg tracking-tight">Scan options</h2>
            </div>
            {draft ? (
              <div className="space-y-5">
                <KnobSlider
                  id="sample-fps"
                  label="Sample FPS"
                  value={Math.round(draft.sample_fps)}
                  display={draft.sample_fps.toFixed(0)}
                  min={1}
                  max={60}
                  marks={["1 light", "12 default", "60 max"]}
                  onChange={(n) => setDraft({ ...draft, sample_fps: n })}
                />
                <KnobSlider
                  id="prefetch"
                  label="Decode prefetch"
                  value={draft.prefetch_depth}
                  display={String(draft.prefetch_depth)}
                  min={1}
                  max={32}
                  onChange={(n) => setDraft({ ...draft, prefetch_depth: n })}
                />
                <div className="grid gap-3 sm:grid-cols-2">
                  <FieldSelect
                    label="Detection size"
                    value={String(draft.det_size)}
                    onChange={(v) =>
                      setDraft({ ...draft, content_preset: "", det_size: Number(v) })
                    }
                    options={[
                      { v: "640", t: "640 · faster" },
                      { v: "960", t: "960 · balanced" },
                      { v: "1280", t: "1280 · quality" },
                    ]}
                  />
                  <FieldSelect
                    label="Detect max edge"
                    value={String(draft.detect_max_edge)}
                    onChange={(v) => setDraft({ ...draft, detect_max_edge: Number(v) })}
                    options={[
                      { v: "1280", t: "1280 · fast 4K" },
                      { v: "1920", t: "1920 · default" },
                      { v: "2560", t: "2560 · sharper" },
                      { v: "0", t: "0 · native" },
                    ]}
                  />
                </div>
                <div className="grid gap-3 sm:grid-cols-3">
                  <NumField
                    id="min-face"
                    label="Min face px"
                    value={draft.min_face_size}
                    min={16}
                    max={256}
                    onChange={(n) =>
                      setDraft({ ...draft, content_preset: "", min_face_size: n })
                    }
                  />
                  <NumField
                    id="cluster-min"
                    label="Min group size"
                    value={draft.cluster_min_size}
                    min={2}
                    max={64}
                    onChange={(n) =>
                      setDraft({ ...draft, content_preset: "", cluster_min_size: n })
                    }
                  />
                  <NumField
                    id="cluster-merge"
                    label="Group merge"
                    value={draft.cluster_merge_sim}
                    min={0.25}
                    max={0.9}
                    step={0.01}
                    onChange={(n) =>
                      setDraft({ ...draft, content_preset: "", cluster_merge_sim: n })
                    }
                  />
                </div>
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  Detect max edge shrinks frames before the model. Live face preview still crops
                  native source pixels.
                </p>

                <div className="border-t border-border/40 pt-5">
                  <h3 className="font-heading text-base tracking-tight">Scene breaks</h3>
                  <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
                    TransNet finds breaks, then a hard-cut snap locks each boundary to the true
                    frame peak (fixes early/late instant cuts). Soft fades uses a gentler
                    threshold for dissolves.
                  </p>
                  <div className="mt-3 grid gap-3 sm:grid-cols-2">
                    <FieldSelect
                      label="Mode"
                      value={draft.scene_cut_detector}
                      onChange={(v) =>
                        setDraft({
                          ...draft,
                          scene_cut_detector: v as typeof draft.scene_cut_detector,
                        })
                      }
                      options={[
                        { v: "auto", t: "Auto" },
                        { v: "transnet", t: "Precise" },
                        {
                          v: "adaptive",
                          t: draft.scene_cut_capabilities?.transnet
                            ? "Soft fades"
                            : "Soft fades · install scene tool",
                        },
                      ]}
                    />
                    <div className="rounded-lg border border-border/40 bg-background/40 px-3 py-2 text-[11px] text-muted-foreground">
                      <p>Requires graphics acceleration</p>
                      <p className="mt-0.5">
                        Scene tool:{" "}
                        {draft.scene_cut_capabilities?.transnet
                          ? "ready"
                          : "Settings → Model vault"}
                      </p>
                    </div>
                  </div>
                  <div className="mt-3 grid gap-3 sm:grid-cols-2">
                    <NumField
                      id="cut-adaptive"
                      label="Soft fade sensitivity"
                      value={draft.scene_cut_adaptive_threshold}
                      min={1}
                      max={10}
                      step={0.1}
                      onChange={(n) =>
                        setDraft({ ...draft, scene_cut_adaptive_threshold: n })
                      }
                    />
                    <NumField
                      id="cut-transnet"
                      label="Precise sensitivity"
                      value={draft.scene_cut_transnet_threshold}
                      min={0.1}
                      max={0.95}
                      step={0.05}
                      onChange={(n) =>
                        setDraft({ ...draft, scene_cut_transnet_threshold: n })
                      }
                    />
                  </div>
                </div>
              </div>
            ) : (
              <LoadingBlock label="Loading scan options…" rows={5} dense />
            )}
          </section>
        </div>

        {/* Paths */}
        <section className="panel-surface overflow-hidden">
          <div className="grid gap-0 md:grid-cols-[1fr_1.4fr]">
            <div className="border-b border-border/40 p-5 md:border-b-0 md:border-r">
              <div className="mb-3 flex items-center gap-2">
                <SlidersHorizontal className="size-4 text-primary" />
                <h2 className="font-heading text-lg tracking-tight">Active defaults</h2>
              </div>
              {settings ? (
                <dl className="grid grid-cols-2 gap-2 text-sm">
                  <Row k="Footage type" v={friendlyBackend(settings.default_backend)} />
                  <Row k="Quality pack" v={friendlyModel(settings.default_backend, settings.default_model)} />
                </dl>
              ) : (
                <p className="text-sm text-muted-foreground">…</p>
              )}
            </div>
            <div className="p-5">
              <div className="mb-3 flex items-center gap-2">
                <HardDrive className="size-4 text-primary" />
                <h2 className="font-heading text-lg tracking-tight">Workspace</h2>
              </div>
              {settings ? (
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
                      Data directory
                    </p>
                    <p className="mt-1 break-all font-mono text-xs leading-relaxed">
                      {settings.data_dir}
                    </p>
                  </div>
                  <CopyButton text={settings.data_dir} label="Copy" toastLabel="Data path copied" />
                </div>
              ) : null}
              <Separator className="my-4" />
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                Some face packs need a commercial license for shipping products. Check your
                rights before release.
              </p>
            </div>
          </div>
        </section>

        {dirty ? (
          <motion.div
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            className="sticky bottom-4 z-10 mx-auto flex w-full max-w-md items-center justify-between gap-3 rounded-2xl border border-primary/35 bg-[oklch(0.12_0.02_280/0.92)] px-4 py-3 shadow-lg backdrop-blur-md"
          >
            <p className="text-xs text-muted-foreground">Unsaved changes</p>
            <Button size="sm" disabled={saving} onClick={() => void save()}>
              <Save className="size-3.5" />
              {saving ? "Saving…" : "Save"}
            </Button>
          </motion.div>
        ) : null}
      </div>
    </div>
  )
}

function KnobSlider({
  id,
  label,
  value,
  display,
  min,
  max,
  marks,
  onChange,
}: {
  id: string
  label: string
  value: number
  display: string
  min: number
  max: number
  marks?: string[]
  onChange: (n: number) => void
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-end justify-between gap-3">
        <Label htmlFor={id}>{label}</Label>
        <span className="timecode text-sm tabular-nums text-primary">{display}</span>
      </div>
      <Slider
        id={id}
        min={min}
        max={max}
        step={1}
        value={[value]}
        onValueChange={(v) => {
          const n = Array.isArray(v) ? v[0] : v
          if (typeof n === "number") onChange(n)
        }}
      />
      {marks ? (
        <div className="flex justify-between text-[10px] uppercase tracking-wider text-muted-foreground">
          {marks.map((m) => (
            <span key={m}>{m}</span>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function FieldSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  options: { v: string; t: string }[]
}) {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <Select value={value} onValueChange={(v) => v && onChange(v)}>
        <SelectTrigger className="bg-background/50">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.v} value={o.v}>
              {o.t}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

function NumField({
  id,
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  id: string
  label: string
  value: number
  min: number
  max: number
  step?: number
  onChange: (n: number) => void
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="number"
        min={min}
        max={max}
        step={step}
        className="bg-background/50"
        value={value}
        onChange={(e) => onChange(Number(e.target.value) || value)}
      />
    </div>
  )
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-lg border border-border/35 bg-background/30 px-3 py-2">
      <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">{k}</dt>
      <dd className="mt-0.5 font-medium capitalize">{v}</dd>
    </div>
  )
}
