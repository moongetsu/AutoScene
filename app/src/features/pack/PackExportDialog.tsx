import { useRef, useState } from "react"
import { CircleNotch, FolderOpen } from "@/components/icons"
import { toast } from "sonner"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { TruncatedText } from "@/components/ui/tooltip"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Select } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { plural } from "@/features/cast/cast"
import { rememberPreset } from "@/features/export/export"
import { formatBytes, shortFormatName } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder } from "@/lib/media"
import { qk, queryClient, useExportDir, usePresets, useSettings } from "@/lib/queries"
import { useUi } from "@/state/ui"
import { type Pack, packExportStarted } from "./use-pack"

/** What a file name can be made of (as_project::naming). */
const TOKENS = [
  { token: "{series}", label: "Series" },
  { token: "{character}", label: "Character" },
  { token: "{episode}", label: "Episode" },
  { token: "{index}", label: "Number" },
  { token: "{tc}", label: "Timecode" },
  { token: "{frame}", label: "Frame" },
]
/** What names one file holding the whole pack: the rest name a shot. */
const FILE_TOKENS = TOKENS.slice(0, 2)

/** Does the one file's name use a part that only names a shot (or no name at all)? */
const fileNameIsWrong = (template: string) => template.trim() === "" || /\{(?!series\}|character\})/.test(template)

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
  const state = pack.state!
  const { naming } = state
  const merge = naming.merge
  const presetId = chosenPreset ?? settings?.exportPresetId
  const preset = presets?.find((p) => p.id === presetId)
  const clips = pack.preview.data?.totals.clips ?? 0
  const names = pack.plan.data?.names ?? []
  const size = pack.plan.data?.sizes.find((s) => s.presetId === presetId)
  const bytes = size?.bytes
  // Bit-Perfect joins only episodes alike: otherwise a file per shot, and why.
  const apart = merge ? (size?.apart ?? null) : null
  const oneFile = merge && apart == null
  const file = oneFile ? pack.plan.data?.file : null
  // A template that doesn't work: the backend says why, under the field it's about.
  const problem = pack.plan.isError ? errorMessage(pack.plan.error) : null
  const fileProblem = merge && fileNameIsWrong(naming.file) ? problem : null
  const namesProblem = fileProblem ? null : problem
  const base = dir ?? defaultDir ?? ""
  const sep = base.includes("\\") ? "\\" : "/"
  // The character's folder as the backend names it (a file per shot only).
  const folder = naming.folder && !oneFile ? pack.plan.data?.folder : null
  const outDir = folder && base ? `${base.replace(/[\\/]+$/, "")}${sep}${folder}` : base
  const shots = plural(clips, "shot")

  const start = async () => {
    const stamp = pack.preview.data?.stamp
    if (!presetId || stamp == null) return
    setBusy(true)
    try {
      const ex = await commands.packExport(pid, character.id, { spec: state.spec, naming, presetId, outDir: dir, stamp })
      // The pack's Export button follows it from now on.
      packExportStarted(pid, character.id, ex.id)
      rememberPreset(settings, presetId)
      onDone()
      toast(oneFile ? `Exporting ${shots} of ${character.label} into one file` : `Exporting ${shots} of ${character.label}`, {
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

  const nameList = names.length > 0 && (
    <span className="flex flex-col gap-0.5 pt-0.5 font-mono text-xs text-fg-2">
      {names.slice(0, 3).map((n, i) => (
        <span key={i} className="truncate" data-selectable>
          {n}
        </span>
      ))}
      {clips > names.length && <span className="text-fg-3">and {(clips - names.length).toLocaleString("en-GB")} more</span>}
    </span>
  )

  return (
    <DialogContent
      title={`Export ${character.label}'s scenepack`}
      description={
        oneFile
          ? `${shots} in one video, each cut frame-exact and checked against the source.`
          : `${shots}, each cut frame-exact and checked against the source.`
      }
      className="w-[min(600px,calc(100vw-32px))]"
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault()
          void start()
        }}
      >
        <label className="flex items-center justify-between gap-4 rounded-lg border border-hairline bg-inset px-3.5 py-3">
          <span>
            <span className="block text-sm text-fg-1">All shots in one file</span>
            <span className="block text-xs text-fg-3">One video with a chapter for each shot, in the pack's order.</span>
          </span>
          <Switch checked={merge} onCheckedChange={(merge) => pack.setNaming({ ...naming, merge })} />
        </label>
        {oneFile && (
          <Field
            label="File name"
            htmlFor="pack-file"
            hint={
              fileProblem ? (
                <span className="text-danger">{fileProblem}</span>
              ) : (
                file &&
                preset && (
                  <span className="block truncate pt-0.5 font-mono text-xs text-fg-2" data-selectable>
                    {file}.{size?.ext ?? preset.container}
                  </span>
                )
              )
            }
          >
            <TemplateInput
              id="pack-file"
              value={naming.file}
              onChange={(f) => pack.setNaming({ ...naming, file: f })}
              tokens={FILE_TOKENS}
              invalid={fileProblem != null}
              // Enter exports: Ctrl/⌘ E, then Enter.
              autoFocus
            />
          </Field>
        )}
        <Field
          label={oneFile ? "Chapter names" : "File names"}
          htmlFor="pack-template"
          hint={namesProblem ? <span className="text-danger">{namesProblem}</span> : nameList}
        >
          <TemplateInput
            id="pack-template"
            value={naming.template}
            onChange={(template) => pack.setNaming({ ...naming, template })}
            tokens={TOKENS}
            invalid={namesProblem != null}
            autoFocus={!oneFile}
          />
        </Field>
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
                {oneFile && bytes != null && bytes > 0 && " The shots are rendered first, so it needs extra free space for a while."}
                {apart && <span className="block pt-1 text-fg-2">{apart}</span>}
                {size?.note && <span className="block pt-1 text-fg-2">{size.note}</span>}
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
          {!oneFile && (
            <label className="mt-3 flex items-center gap-3 text-md text-fg-2">
              <Switch checked={naming.folder} onCheckedChange={(folder) => pack.setNaming({ ...naming, folder })} />
              In a folder named after {character.label}
            </label>
          )}
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
            {oneFile ? `Export ${shots} into one file` : `Export ${shots}`}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}

/** A name template, with buttons that put a name part where the cursor is. */
function TemplateInput({
  id,
  value,
  onChange,
  tokens,
  invalid,
  autoFocus,
}: {
  id: string
  value: string
  onChange: (value: string) => void
  tokens: typeof TOKENS
  invalid: boolean
  autoFocus?: boolean
}) {
  const input = useRef<HTMLInputElement>(null)
  const insert = (token: string) => {
    const el = input.current
    const at = el?.selectionStart ?? value.length
    const to = el?.selectionEnd ?? at
    onChange(value.slice(0, at) + token + value.slice(to))
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(at + token.length, at + token.length)
    })
  }
  return (
    <>
      <Input
        id={id}
        ref={input}
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="font-mono text-sm"
        aria-invalid={invalid}
      />
      <div className="mt-2 flex flex-wrap gap-1.5">
        {tokens.map((t) => (
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
    </>
  )
}
