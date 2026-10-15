import { useEffect, useMemo, useRef, useState } from "react"
import { useNavigate } from "@tanstack/react-router"
import { CircleNotch, FilePlus, FilmSlate, FilmStrip, FolderPlus, Television, X } from "@/components/icons"
import { toast } from "sonner"
import type { ContentKind, ImportCandidate, ProjectKind } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { Segmented } from "@/components/ui/segmented"
import { formatBytes } from "@/lib/format"
import { commands, errorMessage } from "@/lib/ipc"
import { fileStem, pickFolder, pickVideoFiles } from "@/lib/media"
import { cn } from "@/lib/utils"
import { useNewProject } from "@/state/new-project"
import { suggestTitle } from "./suggest-title"

export function NewProjectDialog() {
  const open = useNewProject((s) => s.open)
  const setOpen = useNewProject((s) => s.setOpen)
  const seed = useNewProject((s) => s.seed)
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      {open && <NewProjectForm seed={seed} onDone={() => setOpen(false)} />}
    </Dialog>
  )
}

function NewProjectForm({ seed, onDone }: { seed: string[]; onDone: () => void }) {
  const navigate = useNavigate()
  const [title, setTitle] = useState("")
  const titleTouched = useRef(false)
  const [kind, setKind] = useState<ProjectKind>("series")
  const kindTouched = useRef(false)
  const [content, setContent] = useState<ContentKind>("live")
  // Dropped files (`seed`, which grows when more are dropped on the open
  // dialog) plus picked ones; nothing typed is lost when more arrive.
  const [picked, setPicked] = useState<string[]>([])
  const sources = useMemo(() => [...new Set([...seed, ...picked])], [seed, picked])
  const [scan, setScan] = useState<{ key: string; found: ImportCandidate[] }>({ key: "", found: [] })
  const [removed, setRemoved] = useState<Set<string>>(new Set())
  const [creating, setCreating] = useState(false)
  const scanKey = JSON.stringify([kind, sources])
  const scanning = sources.length > 0 && scan.key !== scanKey
  const candidates = useMemo(() => (sources.length > 0 ? scan.found : []), [sources, scan])

  // Re-scan whenever sources or the project type change (type decides episode parsing).
  useEffect(() => {
    if (sources.length === 0) return
    let stale = false
    commands
      .importScan(sources, kind, null)
      .then((found) => {
        if (stale) return
        setScan({ key: JSON.stringify([kind, sources]), found })
        if (!kindTouched.current) {
          const numbered = found.some((c) => c.episode != null)
          const next: ProjectKind = numbered || found.length > 1 ? "series" : "movie"
          if (next !== kind) setKind(next)
        }
        if (!titleTouched.current && found.length > 0) {
          const first = sources[0]
          const isFile = found.some((c) => c.path === first)
          setTitle(suggestTitle(first, isFile) || fileStem(found[0].fileName))
        }
      })
      .catch((e) => {
        if (stale) return
        setScan({ key: JSON.stringify([kind, sources]), found: [] })
        toast.error("Couldn't read those files", { description: errorMessage(e) })
      })
    return () => {
      stale = true
    }
  }, [sources, kind])

  const chosen = useMemo(() => candidates.filter((c) => !removed.has(c.path)), [candidates, removed])
  const totalBytes = chosen.reduce((n, c) => n + (c.sizeBytes ?? 0), 0)
  const addSources = (paths: string[]) => paths.length && setPicked((s) => [...new Set([...s, ...paths])])

  const create = async () => {
    setCreating(true)
    try {
      const p = await commands.projectCreate({ title: title.trim(), kind, content, files: chosen.map((c) => c.path) })
      onDone()
      void navigate({ to: "/p/$pid/episodes", params: { pid: p.id } })
    } catch (e) {
      toast.error("Couldn't create the project", { description: errorMessage(e) })
    } finally {
      setCreating(false)
    }
  }

  return (
    <DialogContent
      title="New project"
      description="A project is one series or movie. Episodes are recognised and ordered from their file names."
      className="w-[min(760px,calc(100vw-32px))]"
    >
      <div className="grid grid-cols-[1fr_auto_auto] items-end gap-4">
        <Field label="Title" htmlFor="np-title">
          <Input
            id="np-title"
            autoFocus
            value={title}
            placeholder="Series or movie title"
            onChange={(e) => {
              titleTouched.current = true
              setTitle(e.target.value)
            }}
          />
        </Field>
        <Field label="Type">
          <Segmented<ProjectKind>
            aria-label="Project type"
            value={kind}
            onChange={(v) => {
              kindTouched.current = true
              setKind(v)
            }}
            options={[
              { value: "series", label: <><Television /> Series</> },
              { value: "movie", label: <><FilmStrip /> Movie</> },
            ]}
          />
        </Field>
        <Field label="Footage">
          <Segmented<ContentKind>
            aria-label="Footage"
            value={content}
            onChange={setContent}
            options={[
              { value: "live", label: "Live action" },
              { value: "anime", label: "Anime" },
              { value: "mixed", label: "Mixed" },
            ]}
          />
        </Field>
      </div>

      <div className="mt-6 overflow-hidden rounded-2xl border border-white/[0.08] bg-black/25">
        {candidates.length === 0 ? (
          <div className="flex flex-col items-center border-2 border-dashed border-white/[0.08] m-2 rounded-xl px-6 py-12 text-center">
            {scanning ? (
              <CircleNotch className="size-6 animate-spin text-accent" />
            ) : (
              <>
                <div className="flex size-14 items-center justify-center rounded-2xl bg-white/[0.06] text-fg-1 [&_svg]:size-7">
                  <FilmSlate />
                </div>
                <p className="mt-4 text-lg font-medium text-fg-1">Drop a season folder or video files here</p>
                <p className="mt-1 text-sm text-fg-3">MKV, MP4, MOV, TS and more. Sources are never copied or changed.</p>
              </>
            )}
            <div className="mt-6 flex gap-2.5">
              <Button onClick={() => void pickFolder("Choose a season or movie folder").then((p) => p && addSources([p]))}>
                <FolderPlus /> Add folder…
              </Button>
              <Button variant="ghost" onClick={() => void pickVideoFiles().then(addSources)}>
                <FilePlus /> Add files…
              </Button>
            </div>
          </div>
        ) : (
          <>
            <div className="max-h-[320px] overflow-y-auto p-1.5">
              <table className="w-full text-md">
                <tbody>
                  {candidates.map((c) => {
                    const off = removed.has(c.path)
                    return (
                      <tr key={c.path} className={cn("group", off && "opacity-40")}>
                        <td className="w-24 rounded-l-xl py-2 pl-3 group-hover:bg-white/[0.04]">
                          <span className={cn("timecode text-sm", c.label ? "text-fg-1" : "text-fg-3")}>
                            {c.label || "—"}
                          </span>
                        </td>
                        <td className="max-w-0 py-2 group-hover:bg-white/[0.04]">
                          <div className="truncate text-fg-2" title={c.path}>
                            {c.fileName}
                          </div>
                        </td>
                        <td className="timecode w-24 py-2 pr-2 text-right text-sm text-fg-3 group-hover:bg-white/[0.04]">
                          {formatBytes(c.sizeBytes ?? 0)}
                        </td>
                        <td className="w-10 rounded-r-xl py-1 pr-1.5 group-hover:bg-white/[0.04]">
                          <button
                            aria-label={off ? `Include ${c.fileName}` : `Skip ${c.fileName}`}
                            onClick={() =>
                              setRemoved((r) => {
                                const n = new Set(r)
                                if (n.has(c.path)) n.delete(c.path)
                                else n.add(c.path)
                                return n
                              })
                            }
                            className="flex size-7 items-center justify-center rounded-full text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-white/[0.1] hover:text-fg-1 focus-visible:opacity-100"
                          >
                            <X className="size-3.5" />
                          </button>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
            <div className="flex items-center justify-between border-t border-white/[0.06] bg-white/[0.02] px-4 py-2.5">
              <span className="text-sm text-fg-2">
                {chosen.length} {chosen.length === 1 ? "video" : "videos"} · {formatBytes(totalBytes)}
                {scanning && <CircleNotch className="ml-2 inline size-3 animate-spin" />}
              </span>
              <div className="flex gap-1">
                <Button size="sm" variant="ghost" onClick={() => void pickFolder().then((p) => p && addSources([p]))}>
                  <FolderPlus /> Folder…
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void pickVideoFiles().then(addSources)}>
                  <FilePlus /> Files…
                </Button>
              </div>
            </div>
          </>
        )}
      </div>

      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!title.trim() || chosen.length === 0 || creating} onClick={() => void create()}>
          {creating && <CircleNotch className="animate-spin" />}
          Create project
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
