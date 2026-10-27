import { useState } from "react"
import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { CircleNotch, FolderOpen, Info } from "@/components/icons"
import { toast } from "sonner"
import type { ClipRequest } from "@/bindings"
import { Button } from "@/components/ui/button"
import { TruncatedText } from "@/components/ui/tooltip"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Select } from "@/components/ui/select"
import { commands, errorMessage } from "@/lib/ipc"
import { formatBytes, shortFormatName } from "@/lib/format"
import { pickFolder } from "@/lib/media"
import { qk, queryClient, useExportDir, usePresets, useSettings } from "@/lib/queries"
import { useUi } from "@/state/ui"

export type ExportClip = ClipRequest & {
  /** What the user sees: "S01E03 · 00:12:03:05 – 00:12:07:11". */
  summary: string
}

/** Choose preset, name and destination, then queue the export. */
export function ExportDialog({
  projectId,
  clips,
  notice,
  label = null,
  open,
  onOpenChange,
}: {
  projectId: string
  clips: ExportClip[]
  /** Something to check before exporting (a clip that runs to the episode's edge). */
  notice?: string
  /** What it is, for the export history ("Walter · 12 shots"). */
  label?: string | null
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {open && clips.length > 0 && (
        <ExportForm projectId={projectId} clips={clips} notice={notice} label={label} onDone={() => onOpenChange(false)} />
      )}
    </Dialog>
  )
}

function ExportForm({
  projectId,
  clips,
  notice,
  label,
  onDone,
}: {
  projectId: string
  clips: ExportClip[]
  notice?: string
  label: string | null
  onDone: () => void
}) {
  const { data: presets } = usePresets()
  const { data: settings } = useSettings()
  const { data: defaultDir } = useExportDir(projectId)
  const [chosenPreset, setPresetId] = useState<string>()
  const [dir, setDir] = useState<string | null>(null)
  const [name, setName] = useState("")
  const [busy, setBusy] = useState(false)
  const single = clips.length === 1
  const presetId = chosenPreset ?? settings?.exportPresetId

  const preset = presets?.find((p) => p.id === presetId)
  const outDir = dir ?? defaultDir ?? ""
  // The names the files get and the size per format, from the backend.
  const requests = clips.map(({ mediaId, start, end, name }) => ({ mediaId, start, end, name }))
  const plan = useQuery({
    queryKey: ["exportPlan", projectId, requests],
    queryFn: () => commands.exportPlan(projectId, requests),
    placeholderData: keepPreviousData,
    retry: false,
  })
  const suggested = plan.data?.names[0] ?? ""
  const bytes = plan.data?.sizes.find((s) => s.presetId === presetId)?.bytes

  const start = async () => {
    if (!presetId) return
    setBusy(true)
    try {
      const req = {
        presetId,
        outDir: dir,
        clips: clips.map((c) => ({ mediaId: c.mediaId, start: c.start, end: c.end, name: single && name.trim() ? name.trim() : c.name })),
        label,
      }
      await commands.exportStart(projectId, req)
      // Remember the preset for next time.
      if (settings && settings.exportPresetId !== presetId) {
        void commands.settingsUpdate({ ...settings, exportPresetId: presetId }).then((s) => queryClient.setQueryData(qk.settings, s))
      }
      onDone()
      toast(single ? `Exporting “${name.trim() || suggested || "the clip"}”` : `Exporting ${clips.length} clips`, {
        action: { label: "Show", onClick: () => useUi.getState().setActivityOpen(true) },
      })
    } catch (e) {
      toast.error("Couldn't start the export", { description: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <DialogContent
      title={single ? "Export clip" : `Export ${clips.length} clips`}
      description={single ? clips[0].summary : "Each clip is cut frame-exact and checked against the source."}
      className="w-[min(560px,calc(100vw-32px))]"
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault()
          void start()
        }}
      >
        {notice && (
          <p className="flex items-start gap-2.5 rounded-lg border border-white/10 bg-white/[0.04] px-3.5 py-3 text-sm text-fg-2 [&_svg]:mt-0.5 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-1">
            <Info /> {notice}
          </p>
        )}
        {single && (
          <Field label="File name" htmlFor="export-name">
            <Input
              id="export-name"
              autoFocus
              value={name}
              placeholder={suggested}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
        )}
        <Field
          label="Format"
          hint={
            preset && (
              <>
                {preset.description}
                {bytes != null && bytes > 0 && (
                  <span className="text-fg-2">
                    {" "}
                    About {formatBytes(bytes)} in all as {shortFormatName(preset.name)}.
                  </span>
                )}
              </>
            )
          }
        >
          <Select
            aria-label="Export format"
            value={presetId}
            onChange={setPresetId}
            placeholder="Choose a format"
            options={(presets ?? []).map((p) => ({ value: p.id, label: p.name }))}
          />
        </Field>
        <Field label="Destination">
          <div className="flex gap-2">
            <div
              data-selectable
              className="flex h-10 min-w-0 flex-1 items-center rounded-lg border border-soft bg-white/[0.04] px-3 font-mono text-sm text-fg-2"
            >
              <TruncatedText>{outDir || "…"}</TruncatedText>
            </div>
            <Button size="md" className="h-10" onClick={() => void pickFolder("Export to").then((p) => p && setDir(p))}>
              <FolderOpen /> Change…
            </Button>
          </div>
        </Field>
        <DialogFooter className="mt-1">
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" size="lg" variant="primary" disabled={!presetId || busy}>
            {busy && <CircleNotch className="animate-spin" />}
            Export
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}
