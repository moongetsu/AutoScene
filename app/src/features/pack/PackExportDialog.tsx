import { useRef, useState } from "react"
import { CircleNotch, FolderOpen } from "@/components/icons"
import { toast } from "sonner"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Select } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { plural } from "@/features/cast/cast"
import { formatBytes } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder } from "@/lib/media"
import { qk, queryClient, useExportDir, usePresets, useSettings } from "@/lib/queries"
import { useUi } from "@/state/ui"
import type { Pack } from "./use-pack"

/** What a file name can be made of (as_project::naming). */
const TOKENS = [
  { token: "{series}", label: "Series" },
  { token: "{character}", label: "Character" },
  { token: "{episode}", label: "Episode" },
  { token: "{index}", label: "Number" },
  { token: "{tc}", label: "Timecode" },
  { token: "{frame}", label: "Frame" },
]

/** Name the pack's files, choose format and place, and export exactly the shots the grid shows. */
export function PackExportDialog({
  pid,
  character,
  pack,
  open,
  onOpenChange,
}: {
  pid: string
  character: Character
  pack: Pack
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {open && pack.state && <ExportForm pid={pid} character={character} pack={pack} onDone={() => onOpenChange(false)} />}
    </Dialog>
  )
}

function ExportForm({ pid, character, pack, onDone }: { pid: string; character: Character; pack: Pack; onDone: () => void }) {
  const { data: presets } = usePresets()
  const { data: settings } = useSettings()
  const { data: defaultDir } = useExportDir(pid)
  const [chosenPreset, setPresetId] = useState<string>()
  const [dir, setDir] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const template = useRef<HTMLInputElement>(null)
  const state = pack.state!
  const { naming } = state
  const presetId = chosenPreset ?? settings?.exportPresetId
  const preset = presets?.find((p) => p.id === presetId)
  const clips = pack.preview.data?.totals.clips ?? 0
  const names = pack.plan.data?.names ?? []
  const bytes = pack.plan.data?.sizes.find((s) => s.presetId === presetId)?.bytes
  // A template that doesn't work: the backend says why.
  const problem = pack.plan.isError ? errorMessage(pack.plan.error) : null
  const base = dir ?? defaultDir ?? ""
  const sep = base.includes("\\") ? "\\" : "/"
  // The character's folder as the backend names it.
  const folder = naming.folder ? pack.plan.data?.folder : null
  const outDir = folder && base ? `${base.replace(/[\\/]+$/, "")}${sep}${folder}` : base

  const insert = (token: string) => {
    const el = template.current
    const at = el?.selectionStart ?? naming.template.length
    const to = el?.selectionEnd ?? at
    const next = naming.template.slice(0, at) + token + naming.template.slice(to)
    pack.setNaming({ ...naming, template: next })
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(at + token.length, at + token.length)
    })
  }

  const start = async () => {
    const stamp = pack.preview.data?.stamp
    if (!presetId || stamp == null) return
    setBusy(true)
    try {
      await commands.packExport(pid, character.id, { spec: state.spec, naming, presetId, outDir: dir, stamp })
      if (settings && settings.exportPresetId !== presetId) {
        void commands.settingsUpdate({ ...settings, exportPresetId: presetId }).then((s) => queryClient.setQueryData(qk.settings, s))
      }
      onDone()
      toast(`Exporting ${plural(clips, "shot")} of ${character.label}`, {
        action: { label: "Show", onClick: () => useUi.getState().setActivityOpen(true) },
      })
    } catch (e) {
      toast.error("Couldn't start the export", { description: errorMessage(e) })
      // Refused because the pack changed (a file gone, an analysis done):
      // show it as it is now.
      void queryClient.invalidateQueries({ queryKey: qk.pack(pid, character.id) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <DialogContent
      title={`Export ${character.label}'s scenepack`}
      description={`${plural(clips, "shot")}, each cut frame-exact and checked against the source.`}
      className="w-[min(600px,calc(100vw-32px))]"
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault()
          void start()
        }}
      >
        <Field
          label="File names"
          htmlFor="pack-template"
          hint={
            problem ? (
              <span className="text-danger">{problem}</span>
            ) : (
              names.length > 0 && (
                <span className="flex flex-col gap-0.5 pt-0.5 font-mono text-xs text-fg-2">
                  {names.slice(0, 3).map((n, i) => (
                    <span key={i} className="truncate" data-selectable>
                      {n}
                    </span>
                  ))}
                  {clips > names.length && (
                    <span className="text-fg-3">and {(clips - names.length).toLocaleString("en-GB")} more</span>
                  )}
                </span>
              )
            )
          }
        >
          <Input
            id="pack-template"
            ref={template}
            // Enter exports: Ctrl/⌘ E, then Enter.
            autoFocus
            value={naming.template}
            onChange={(e) => pack.setNaming({ ...naming, template: e.target.value })}
            className="font-mono text-sm"
            aria-invalid={problem != null}
          />
          <div className="mt-2 flex flex-wrap gap-1.5">
            {TOKENS.map((t) => (
              <button
                key={t.token}
                type="button"
                onClick={() => insert(t.token)}
                className="rounded-full border border-white/10 bg-white/[0.04] px-2.5 text-xs leading-6 text-fg-2 hover:border-white/20 hover:text-fg-1"
              >
                {t.label}
              </button>
            ))}
          </div>
        </Field>
        <Field
          label="Format"
          hint={
            preset && (
              <>
                {preset.description}
                {bytes != null && bytes > 0 && <span className="text-fg-2"> About {formatBytes(bytes)} in all.</span>}
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
              title={outDir}
              className="flex h-10 min-w-0 flex-1 items-center rounded-lg border border-soft bg-white/[0.04] px-3 font-mono text-sm text-fg-2"
            >
              <span className="truncate">{outDir || "…"}</span>
            </div>
            <Button size="md" className="h-10" onClick={() => void pickFolder("Export to").then((p) => p && setDir(p))}>
              <FolderOpen /> Change…
            </Button>
          </div>
          <label className="mt-3 flex items-center gap-3 text-md text-fg-2">
            <Switch checked={naming.folder} onCheckedChange={(folder) => pack.setNaming({ ...naming, folder })} />
            In a folder named after {character.label}
          </label>
        </Field>
        <DialogFooter className="mt-1">
          <Button variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            type="submit"
            size="lg"
            variant="primary"
            disabled={!presetId || busy || clips === 0 || problem != null || !pack.current}
          >
            {busy && <CircleNotch className="animate-spin" />}
            Export {plural(clips, "shot")}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}
