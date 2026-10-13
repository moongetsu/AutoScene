import { useState } from "react"
import { FolderOpen, Loader2 } from "lucide-react"
import { toast } from "sonner"
import type { ClipRequest } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Select } from "@/components/ui/select"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder } from "@/lib/media"
import { qk, queryClient, useExportDir, usePresets, useSettings } from "@/lib/queries"
import { useUi } from "@/state/ui"

export type ExportClip = ClipRequest & {
  /** What the user sees: "S01E03 · 00:12:03:05 – 00:12:07:11". */
  summary: string
  /** Default file name the backend would choose (shown as placeholder). */
  suggestedName: string
}

/** Choose preset, name and destination, then queue the export. */
export function ExportDialog({
  projectId,
  clips,
  open,
  onOpenChange,
}: {
  projectId: string
  clips: ExportClip[]
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {open && clips.length > 0 && <ExportForm projectId={projectId} clips={clips} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

function ExportForm({ projectId, clips, onDone }: { projectId: string; clips: ExportClip[]; onDone: () => void }) {
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

  const start = async () => {
    if (!presetId) return
    setBusy(true)
    try {
      const req = {
        presetId,
        outDir: dir,
        clips: clips.map((c) => ({ mediaId: c.mediaId, start: c.start, end: c.end, name: single && name.trim() ? name.trim() : null })),
      }
      await commands.exportStart(projectId, req)
      // Remember the preset for next time.
      if (settings && settings.exportPresetId !== presetId) {
        void commands.settingsUpdate({ ...settings, exportPresetId: presetId }).then((s) => queryClient.setQueryData(qk.settings, s))
      }
      onDone()
      toast(single ? `Exporting “${name.trim() || clips[0].suggestedName}”` : `Exporting ${clips.length} clips`, {
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
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault()
          void start()
        }}
      >
        {single && (
          <Field label="File name" htmlFor="export-name">
            <Input
              id="export-name"
              autoFocus
              value={name}
              placeholder={clips[0].suggestedName}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
        )}
        <Field label="Format" hint={preset?.description}>
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
              title={outDir}
              className="flex h-8 min-w-0 flex-1 items-center rounded-md border border-hairline bg-inset px-2.5 font-mono text-xs text-fg-2"
            >
              <span className="truncate">{outDir || "…"}</span>
            </div>
            <Button onClick={() => void pickFolder("Export to").then((p) => p && setDir(p))}>
              <FolderOpen /> Change…
            </Button>
          </div>
        </Field>
        <DialogFooter className="mt-1">
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!presetId || busy}>
            {busy && <Loader2 className="animate-spin" />}
            Export
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}
