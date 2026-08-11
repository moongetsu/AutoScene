import { useEffect, useMemo, useState } from "react"
import { motion } from "motion/react"
import {
  Boxes,
  Cpu,
  Gauge,
  MonitorCog,
  RefreshCw,
  RotateCcw,
  Save,
  ScanFace,
  Scissors,
  SlidersHorizontal,
  Undo2,
} from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { PageHeader } from "@/components/app/PageHeader"
import { CastTeachPanel } from "@/components/settings/CastTeachPanel"
import { ModelVault } from "@/components/settings/ModelVault"
import { UpdatesPanel } from "@/components/settings/UpdatesPanel"
import {
  ChoiceTiles,
  InfoRow,
  NumberControl,
  SelectControl,
  SettingRow,
  SettingsGroup,
  SliderControl,
} from "@/components/settings/SettingsPrimitives"
import { CopyButton } from "@/components/shared/CopyButton"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import {
  CONTENT_PRESET_FALLBACK,
  SETTINGS_DEFAULTS,
  type AppSettings,
  type ContentPresetId,
  type FaceSelectMode,
} from "@/lib/api"
import { useProjects } from "@/state/project"
import { useAppSettings } from "@/state/settings"
import { friendlyBackend, friendlyGpuLine, friendlyModel, friendlyError } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type SectionId = "general" | "faces" | "quality" | "models" | "system"

const SECTIONS: {
  id: SectionId
  label: string
  blurb: string
  icon: typeof SlidersHorizontal
}[] = [
  {
    id: "general",
    label: "General",
    blurb: "What you are scanning",
    icon: SlidersHorizontal,
  },
  { id: "faces", label: "Faces", blurb: "Who gets picked up", icon: ScanFace },
  { id: "quality", label: "Scan quality", blurb: "Speed vs detail", icon: Gauge },
  { id: "models", label: "Models", blurb: "Downloads and packs", icon: Boxes },
  { id: "system", label: "System", blurb: "Device, files, updates", icon: MonitorCog },
]

const FACE_MODES: {
  id: FaceSelectMode
  label: string
  blurb: string
}[] = [
  {
    id: "all",
    label: "Everyone in frame",
    blurb: "Keeps every face found. Best cast coverage, slowest.",
  },
  {
    id: "largest",
    label: "Main subject only",
    blurb: "One face per frame — the biggest one. Good for interviews.",
  },
  {
    id: "relative",
    label: "Bigger faces only",
    blurb: "Skips background faces below a size you choose.",
  },
  {
    id: "top_n",
    label: "A few per frame",
    blurb: "Keeps the largest few faces. Good for two-shots.",
  },
]

/** Fields each section owns — drives the per-section "Reset" button. */
const SECTION_FIELDS: Record<SectionId, (keyof AppSettings)[]> = {
  general: ["content_preset", "default_backend", "default_model"],
  faces: ["face_select_mode", "face_relative_pct", "face_top_n"],
  quality: [
    "sample_fps",
    "det_size",
    "detect_max_edge",
    "prefetch_depth",
    "min_face_size",
    "cluster_min_size",
    "cluster_merge_sim",
    "scene_cut_detector",
    "scene_cut_adaptive_threshold",
    "scene_cut_transnet_threshold",
  ],
  models: [],
  system: [],
}

const SAVED_FIELDS = [
  "sample_fps",
  "det_size",
  "detect_max_edge",
  "prefetch_depth",
  "min_face_size",
  "cluster_min_size",
  "cluster_merge_sim",
  "content_preset",
  "default_backend",
  "default_model",
  "face_select_mode",
  "face_relative_pct",
  "face_top_n",
  "scene_cut_detector",
  "scene_cut_adaptive_threshold",
  "scene_cut_transnet_threshold",
] as const satisfies readonly (keyof AppSettings)[]

export function SettingsPage() {
  const { gpu, gpuReady, refreshGpu } = useProjects()
  const { settings, settingsError, refreshSettings, saveSettings } = useAppSettings()
  const [draft, setDraft] = useState<AppSettings | null>(null)
  const [section, setSection] = useState<SectionId>("general")
  const [advanced, setAdvanced] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (settings) setDraft(settings)
  }, [settings])

  useEffect(() => {
    void refreshSettings()
  }, [refreshSettings])

  const changed = useMemo(() => {
    const set = new Set<keyof AppSettings>()
    if (!settings || !draft) return set
    for (const key of SAVED_FIELDS) {
      if (settings[key] !== draft[key]) set.add(key)
    }
    return set
  }, [settings, draft])

  const dirty = changed.size > 0

  const patch = (next: Partial<AppSettings>) =>
    setDraft((d) => (d ? { ...d, ...next } : d))

  const resetField = (key: keyof typeof SETTINGS_DEFAULTS) =>
    patch({ [key]: SETTINGS_DEFAULTS[key] } as Partial<AppSettings>)

  const resetSection = (id: SectionId) => {
    if (!draft) return
    const next: Partial<AppSettings> = {}
    for (const key of SECTION_FIELDS[id]) {
      if (key in SETTINGS_DEFAULTS) {
        Object.assign(next, {
          [key]: SETTINGS_DEFAULTS[key as keyof typeof SETTINGS_DEFAULTS],
        })
      }
    }
    if (id === "general") {
      // Back to the shipped footage type, knobs included.
      const presets = draft.content_presets?.length
        ? draft.content_presets
        : CONTENT_PRESET_FALLBACK
      const base = presets.find((p) => p.id === "live_action") ?? presets[0]
      if (base) {
        next.content_preset = base.id
        next.cluster_merge_sim = base.cluster_merge_sim
        next.cluster_min_size = base.cluster_min_size
        next.min_face_size = base.min_face_size
        next.det_size = base.det_size
        next.default_backend = base.default_backend || draft.default_backend
        next.default_model = base.default_model || draft.default_model
      }
    }
    patch(next)
  }

  const save = async () => {
    if (!draft) return
    setSaving(true)
    try {
      const next = await saveSettings(
        Object.fromEntries(SAVED_FIELDS.map((k) => [k, draft[k]])) as Parameters<
          typeof saveSettings
        >[0],
      )
      setDraft(next)
      toast.success("Settings saved. Your next scan uses them.")
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setSaving(false)
    }
  }

  const sectionDirty = (id: SectionId) =>
    SECTION_FIELDS[id].some((k) => changed.has(k))

  const current = SECTIONS.find((s) => s.id === section)!

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto">
      <PageHeader
        eyebrow="Settings"
        title="Settings"
        description="Pick what you are scanning, then leave the rest alone. Changes apply to your next scan."
        actions={
          <>
            <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-border/45 bg-background/40 px-3 py-1.5">
              <Switch
                id="advanced-mode"
                checked={advanced}
                onCheckedChange={setAdvanced}
              />
              <span className="text-xs text-muted-foreground">Advanced</span>
            </label>
            <Button size="sm" disabled={!dirty || saving || !draft} onClick={() => void save()}>
              <Save className="size-3.5" />
              {saving ? "Saving…" : dirty ? `Save ${changed.size}` : "Saved"}
            </Button>
          </>
        }
      />

      <div className="mx-auto flex w-full max-w-6xl flex-col gap-5 p-5 lg:flex-row lg:gap-8 lg:p-8">
        {/* Section rail — horizontal chips on small screens */}
        <nav
          aria-label="Settings sections"
          className="-mx-1 flex shrink-0 gap-1.5 overflow-x-auto px-1 pb-1 lg:sticky lg:top-4 lg:mx-0 lg:w-56 lg:flex-col lg:self-start lg:overflow-visible lg:px-0"
        >
          {SECTIONS.map((s) => {
            const Icon = s.icon
            const active = s.id === section
            return (
              <button
                key={s.id}
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => setSection(s.id)}
                className={cn(
                  "group flex shrink-0 items-center gap-2.5 rounded-xl border px-3 py-2 text-left transition lg:w-full lg:py-2.5",
                  active
                    ? "border-primary/45 bg-primary/[0.07] text-foreground"
                    : "border-transparent text-muted-foreground hover:border-border/50 hover:bg-card/50 hover:text-foreground",
                )}
              >
                <Icon className={cn("size-4 shrink-0", active && "text-primary")} />
                <span className="min-w-0">
                  <span className="block whitespace-nowrap text-sm font-medium">
                    {s.label}
                  </span>
                  <span className="hidden text-[11px] leading-tight text-muted-foreground lg:block">
                    {s.blurb}
                  </span>
                </span>
                {sectionDirty(s.id) ? (
                  <span
                    aria-label="Unsaved changes"
                    className="ml-auto size-1.5 shrink-0 rounded-full bg-primary"
                  />
                ) : null}
              </button>
            )
          })}
        </nav>

        <motion.div
          key={section}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ type: "spring", stiffness: 260, damping: 28 }}
          className="flex min-w-0 flex-1 flex-col gap-5"
        >
          {section !== "models" && SECTION_FIELDS[section].length > 0 ? (
            <div className="flex items-center justify-between gap-3 px-0.5">
              <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
                {current.label}
              </p>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="text-muted-foreground"
                disabled={!draft}
                onClick={() => resetSection(section)}
              >
                <RotateCcw className="size-3" />
                Reset this page
              </Button>
            </div>
          ) : null}

          {section === "general" ? (
            <GeneralSection
              draft={draft}
              settings={settings}
              settingsError={settingsError}
              changed={changed}
              patch={patch}
            />
          ) : null}

          {section === "faces" ? (
            <FacesSection
              draft={draft}
              changed={changed}
              patch={patch}
              resetField={resetField}
            />
          ) : null}

          {section === "quality" ? (
            <QualitySection
              draft={draft}
              advanced={advanced}
              changed={changed}
              patch={patch}
              resetField={resetField}
              onShowAdvanced={() => setAdvanced(true)}
            />
          ) : null}

          {section === "models" ? <ModelVault /> : null}

          {section === "system" ? (
            <SystemSection
              settings={settings}
              gpu={gpu}
              gpuReady={gpuReady}
              refreshing={refreshing}
              onRefresh={async () => {
                setRefreshing(true)
                try {
                  await refreshGpu()
                } finally {
                  setRefreshing(false)
                }
              }}
            />
          ) : null}
        </motion.div>
      </div>

      {dirty ? (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="sticky bottom-4 z-10 mx-auto mb-4 flex w-[min(28rem,calc(100%-2.5rem))] items-center justify-between gap-3 rounded-2xl border border-primary/35 bg-[oklch(0.12_0.02_280/0.92)] px-4 py-3 shadow-lg backdrop-blur-md"
        >
          <p className="text-xs text-muted-foreground">
            {changed.size} unsaved {changed.size === 1 ? "change" : "changes"}
          </p>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="ghost"
              disabled={saving}
              onClick={() => settings && setDraft(settings)}
            >
              <Undo2 className="size-3.5" />
              Discard
            </Button>
            <Button size="sm" disabled={saving} onClick={() => void save()}>
              <Save className="size-3.5" />
              {saving ? "Saving…" : "Save"}
            </Button>
          </div>
        </motion.div>
      ) : null}
    </div>
  )
}

function GeneralSection({
  draft,
  settings,
  settingsError,
  changed,
  patch,
}: {
  draft: AppSettings | null
  settings: AppSettings | null
  settingsError: string | null
  changed: Set<keyof AppSettings>
  patch: (next: Partial<AppSettings>) => void
}) {
  const presets = draft?.content_presets?.length
    ? draft.content_presets
    : CONTENT_PRESET_FALLBACK

  return (
    <>
      <SettingsGroup
        title="What are you scanning?"
        description="This one choice sets the face model and grouping for you. Most people never need anything else on this page."
        aside={
          draft && !draft.content_preset ? (
            <Badge variant="secondary" className="text-amber-400/90">
              Custom
            </Badge>
          ) : null
        }
      >
        <SettingRow
          label="Footage type"
          hint="Pick the one closest to your source. Changing it overwrites the tuning knobs under Scan quality."
          changed={changed.has("content_preset")}
          full
        >
          {draft ? (
            <ChoiceTiles
              columns={3}
              value={draft.content_preset || ""}
              options={presets.map((p) => ({
                id: p.id as ContentPresetId,
                label: p.label,
                blurb: p.blurb,
                meta: `${friendlyBackend(p.default_backend)} · ${friendlyModel(
                  p.default_backend,
                  p.default_model,
                )}`,
              }))}
              onChange={(id) => {
                const preset = presets.find((p) => p.id === id)
                if (!preset) return
                patch({
                  content_preset: preset.id,
                  cluster_merge_sim: preset.cluster_merge_sim,
                  cluster_min_size: preset.cluster_min_size,
                  min_face_size: preset.min_face_size,
                  det_size: preset.det_size,
                  default_backend: preset.default_backend || draft.default_backend,
                  default_model: preset.default_model || draft.default_model,
                })
              }}
            />
          ) : (
            <LoadingBlock
              label={
                settingsError
                  ? "Could not load presets — is AutoScene running?"
                  : "Loading presets…"
              }
              rows={2}
              dense
            />
          )}
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup
        title="In use right now"
        description="What your next scan will run with, after your last save."
      >
        <InfoRow
          label="Footage type"
          value={settings ? friendlyBackend(settings.default_backend) : "…"}
        />
        <InfoRow
          label="Quality pack"
          value={
            settings ? friendlyModel(settings.default_backend, settings.default_model) : "…"
          }
        />
      </SettingsGroup>
    </>
  )
}

function FacesSection({
  draft,
  changed,
  patch,
  resetField,
}: {
  draft: AppSettings | null
  changed: Set<keyof AppSettings>
  patch: (next: Partial<AppSettings>) => void
  resetField: (key: keyof typeof SETTINGS_DEFAULTS) => void
}) {
  return (
    <>
      <SettingsGroup
        title="Which faces to keep"
        description="Fewer faces means a faster scan and a shorter cast list. More faces catches extras and background characters."
      >
        <SettingRow
          label="Faces per frame"
          hint="Not sure? Leave it on Everyone in frame."
          changed={changed.has("face_select_mode")}
          full
        >
          {draft ? (
            <ChoiceTiles
              value={draft.face_select_mode}
              options={FACE_MODES.map((m) => ({
                id: m.id,
                label: m.label,
                blurb: m.blurb,
              }))}
              onChange={(id) => patch({ face_select_mode: id })}
            />
          ) : (
            <LoadingBlock label="Loading face options…" rows={2} dense />
          )}
        </SettingRow>

        {draft?.face_select_mode === "relative" ? (
          <SettingRow
            label="How much smaller than the biggest face"
            hint="A face is kept when it is at least this share of the largest face in the same frame."
            htmlFor="relative-pct"
            changed={changed.has("face_relative_pct")}
            onReset={() => resetField("face_relative_pct")}
          >
            <SliderControl
              id="relative-pct"
              min={1}
              max={100}
              value={Math.round(draft.face_relative_pct)}
              display={`${Math.round(draft.face_relative_pct)}%`}
              minLabel="Keep more"
              maxLabel="Keep fewer"
              onChange={(n) => patch({ face_relative_pct: n })}
            />
          </SettingRow>
        ) : null}

        {draft?.face_select_mode === "top_n" ? (
          <SettingRow
            label="How many faces per frame"
            hint="The largest faces are kept, the rest are ignored."
            htmlFor="top-n"
            changed={changed.has("face_top_n")}
            onReset={() => resetField("face_top_n")}
          >
            <NumberControl
              id="top-n"
              value={draft.face_top_n}
              min={1}
              max={12}
              suffix="faces"
              onChange={(n) => patch({ face_top_n: Math.max(1, Math.min(12, n)) })}
            />
          </SettingRow>
        ) : null}
      </SettingsGroup>

      <CastTeachPanel />
    </>
  )
}

function QualitySection({
  draft,
  advanced,
  changed,
  patch,
  resetField,
  onShowAdvanced,
}: {
  draft: AppSettings | null
  advanced: boolean
  changed: Set<keyof AppSettings>
  patch: (next: Partial<AppSettings>) => void
  resetField: (key: keyof typeof SETTINGS_DEFAULTS) => void
  onShowAdvanced: () => void
}) {
  if (!draft) {
    return (
      <div className="panel-surface px-5 py-5">
        <LoadingBlock label="Loading scan options…" rows={5} dense />
      </div>
    )
  }

  const hiddenNote = advanced ? null : (
    <button
      type="button"
      onClick={onShowAdvanced}
      className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
    >
      More fine-tuning is hidden. Turn on Advanced in the header to show it.
    </button>
  )

  return (
    <>
      <SettingsGroup
        title="Speed vs detail"
        description="Higher settings find more faces in fast-moving footage and cost scan time."
        footer={hiddenNote}
      >
        <SettingRow
          label="Frames checked each second"
          hint="Low is quick but can miss faces in fast cuts. High is thorough and slow."
          htmlFor="sample-fps"
          changed={changed.has("sample_fps")}
          onReset={() => resetField("sample_fps")}
        >
          <SliderControl
            id="sample-fps"
            min={1}
            max={60}
            value={Math.round(draft.sample_fps)}
            display={`${Math.round(draft.sample_fps)} fps`}
            minLabel="Fast"
            maxLabel="Thorough"
            onChange={(n) => patch({ sample_fps: n })}
          />
        </SettingRow>

        <SettingRow
          label="Face detail"
          hint="How closely each frame is inspected. Sharper catches small faces and runs slower."
          changed={changed.has("det_size")}
          onReset={() => resetField("det_size")}
        >
          <SelectControl
            ariaLabel="Face detail"
            value={String(draft.det_size)}
            onChange={(v) => patch({ content_preset: "", det_size: Number(v) })}
            options={[
              { value: "640", label: "Faster" },
              { value: "960", label: "Balanced" },
              { value: "1280", label: "Sharper (default)" },
            ]}
          />
        </SettingRow>

        {advanced ? (
          <SettingRow
            label="Working resolution"
            hint="Frames are shrunk to this longest side before faces are found. Previews still use full-size source pixels."
            changed={changed.has("detect_max_edge")}
            onReset={() => resetField("detect_max_edge")}
          >
            <SelectControl
              ariaLabel="Working resolution"
              value={String(draft.detect_max_edge)}
              onChange={(v) => patch({ detect_max_edge: Number(v) })}
              options={[
                { value: "1280", label: "1280 · fastest on 4K" },
                { value: "1920", label: "1920 · default" },
                { value: "2560", label: "2560 · sharper" },
                { value: "0", label: "Full size · slowest" },
              ]}
            />
          </SettingRow>
        ) : null}

        {advanced ? (
          <SettingRow
            label="Video read-ahead"
            hint="How many frames are queued up while scanning. Raise it only if your drive is fast and the scan looks starved."
            htmlFor="prefetch"
            changed={changed.has("prefetch_depth")}
            onReset={() => resetField("prefetch_depth")}
          >
            <SliderControl
              id="prefetch"
              min={1}
              max={32}
              value={draft.prefetch_depth}
              display={String(draft.prefetch_depth)}
              minLabel="Less memory"
              maxLabel="More memory"
              onChange={(n) => patch({ prefetch_depth: n })}
            />
          </SettingRow>
        ) : null}
      </SettingsGroup>

      {advanced ? (
        <SettingsGroup
          title="Grouping characters"
          description="How found faces get merged into one character in Gallery. Only change these if characters keep splitting apart or blending together."
        >
          <SettingRow
            label="Smallest face to keep"
            hint="Faces smaller than this are treated as background and skipped."
            htmlFor="min-face"
            changed={changed.has("min_face_size")}
            onReset={() => resetField("min_face_size")}
          >
            <NumberControl
              id="min-face"
              value={draft.min_face_size}
              min={16}
              max={256}
              suffix="px"
              onChange={(n) => patch({ content_preset: "", min_face_size: n })}
            />
          </SettingRow>

          <SettingRow
            label="Fewest faces per character"
            hint="A character needs at least this many face finds to show up. Raise it to hide one-off extras."
            htmlFor="cluster-min"
            changed={changed.has("cluster_min_size")}
            onReset={() => resetField("cluster_min_size")}
          >
            <NumberControl
              id="cluster-min"
              value={draft.cluster_min_size}
              min={2}
              max={64}
              suffix="faces"
              onChange={(n) => patch({ content_preset: "", cluster_min_size: n })}
            />
          </SettingRow>

          <SettingRow
            label="Grouping strength"
            hint="Lower merges lookalikes into one character. Higher keeps similar faces apart."
            htmlFor="cluster-merge"
            changed={changed.has("cluster_merge_sim")}
            onReset={() => resetField("cluster_merge_sim")}
          >
            <SliderControl
              id="cluster-merge"
              min={0.25}
              max={0.9}
              step={0.01}
              value={draft.cluster_merge_sim}
              display={draft.cluster_merge_sim.toFixed(2)}
              minLabel="Merge more"
              maxLabel="Split more"
              onChange={(n) => patch({ content_preset: "", cluster_merge_sim: n })}
            />
          </SettingRow>
        </SettingsGroup>
      ) : null}

      <SettingsGroup
        title="Scene breaks"
        description="How AutoScene decides where one shot ends and the next begins."
        aside={
          <Badge variant="secondary" className="gap-1.5">
            <Scissors className="size-3" />
            {draft.scene_cut_capabilities?.transnet ? "Scene tool ready" : "Scene tool missing"}
          </Badge>
        }
        footer={
          draft.scene_cut_capabilities?.transnet ? null : (
            <p className="text-xs text-muted-foreground">
              Precise mode needs the scene tool. Install it under Models.
            </p>
          )
        }
      >
        <SettingRow
          label="Cut style"
          hint="Frame delta (HSV adaptive) is the default — hard cuts, no AI. Precise uses TransNet for dissolves. Auto tries Precise then falls back."
          changed={changed.has("scene_cut_detector")}
          onReset={() => resetField("scene_cut_detector")}
        >
          <SelectControl
            ariaLabel="Cut style"
            value={draft.scene_cut_detector}
            onChange={(v) =>
              patch({ scene_cut_detector: v as AppSettings["scene_cut_detector"] })
            }
            options={[
              { value: "adaptive", label: "Frame delta (recommended)" },
              { value: "auto", label: "Auto (AI then fallback)" },
              {
                value: "transnet",
                label: draft.scene_cut_capabilities?.transnet
                  ? "Precise (AI)"
                  : "Precise (AI) · needs scene tool",
              },
            ]}
          />
        </SettingRow>

        {advanced ? (
          <SettingRow
            label="Frame delta sensitivity"
            hint="Higher finds more hard cuts (and more false ones on fast motion)."
            htmlFor="cut-adaptive"
            changed={changed.has("scene_cut_adaptive_threshold")}
            onReset={() => resetField("scene_cut_adaptive_threshold")}
          >
            <NumberControl
              id="cut-adaptive"
              value={draft.scene_cut_adaptive_threshold}
              min={1}
              max={10}
              step={0.1}
              onChange={(n) => patch({ scene_cut_adaptive_threshold: n })}
            />
          </SettingRow>
        ) : null}

        {advanced ? (
          <SettingRow
            label="Precise sensitivity"
            hint="Lower reports more cuts. Higher keeps only the obvious ones."
            htmlFor="cut-transnet"
            changed={changed.has("scene_cut_transnet_threshold")}
            onReset={() => resetField("scene_cut_transnet_threshold")}
          >
            <NumberControl
              id="cut-transnet"
              value={draft.scene_cut_transnet_threshold}
              min={0.1}
              max={0.95}
              step={0.05}
              onChange={(n) => patch({ scene_cut_transnet_threshold: n })}
            />
          </SettingRow>
        ) : null}
      </SettingsGroup>
    </>
  )
}

function SystemSection({
  settings,
  gpu,
  gpuReady,
  refreshing,
  onRefresh,
}: {
  settings: AppSettings | null
  gpu: ReturnType<typeof useProjects>["gpu"]
  gpuReady: boolean
  refreshing: boolean
  onRefresh: () => void | Promise<void>
}) {
  return (
    <>
      <UpdatesPanel />

      <SettingsGroup
        title="Device"
        description="AutoScene needs graphics acceleration to scan at a usable speed."
        aside={
          <Button variant="outline" size="sm" disabled={refreshing} onClick={() => void onRefresh()}>
            <RefreshCw className={cn("size-3.5", refreshing && "animate-spin")} />
            Check again
          </Button>
        }
      >
        {gpuReady && gpu ? (
          <>
            <InfoRow
              label="Graphics"
              value={
                <Badge variant={gpu.available ? "default" : "secondary"}>
                  {gpu.available ? "Ready" : "Unavailable"}
                </Badge>
              }
            />
            <InfoRow
              label="Device"
              value={friendlyGpuLine(gpu.available, gpu.name, gpu.detail)}
            />
          </>
        ) : (
          <div className="px-5 py-4">
            <LoadingBlock label="Checking graphics…" rows={2} dense />
          </div>
        )}
      </SettingsGroup>

      <SettingsGroup
        title="Files"
        description="Projects, portraits, and exports all live here."
        footer={
          <p className="flex items-start gap-2 text-xs leading-relaxed text-muted-foreground">
            <Cpu className="mt-0.5 size-3.5 shrink-0" />
            Some face packs need a commercial license for shipping products. Check your rights
            before release.
          </p>
        }
      >
        {settings ? (
          <InfoRow
            label="Data folder"
            mono
            value={settings.data_dir}
            action={
              <CopyButton
                text={settings.data_dir}
                label="Copy"
                toastLabel="Data path copied"
              />
            }
          />
        ) : (
          <div className="px-5 py-4">
            <LoadingBlock label="Loading paths…" rows={1} dense />
          </div>
        )}
      </SettingsGroup>
    </>
  )
}
