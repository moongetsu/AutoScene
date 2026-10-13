import { useEffect, useMemo, useRef, useState } from "react"
import { useNavigate } from "@tanstack/react-router"
import { Clapperboard, FilePlus2, Film, FolderPlus, Loader2, Tv, X } from "lucide-react"
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
      {open && <NewProjectForm key={seed.join("|")} seed={seed} onDone={() => setOpen(false)} />}
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
  const [sources, setSources] = useState<string[]>(seed)
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
  const addSources = (paths: string[]) => paths.length && setSources((s) => [...new Set([...s, ...paths])])

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
      className="w-[min(720px,calc(100vw-32px))]"
    >
      <div className="grid grid-cols-[1fr_auto_auto] items-end gap-3">
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
              { value: "series", label: <><Tv /> Series</> },
              { value: "movie", label: <><Film /> Movie</> },
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

      <div className="mt-4 overflow-hidden rounded-lg border border-hairline bg-inset">
        {candidates.length === 0 ? (
          <div className="flex flex-col items-center px-6 py-10 text-center">
            {scanning ? (
              <Loader2 className="size-5 animate-spin text-fg-3" />
            ) : (
              <>
                <div className="flex size-10 items-center justify-center rounded-lg border border-hairline bg-panel text-fg-3 shadow-1">
                  <Clapperboard className="size-5" />
                </div>
                <p className="mt-3 text-sm text-fg-1">Drop a season folder or video files here</p>
                <p className="mt-0.5 text-xs text-fg-3">MKV, MP4, MOV, TS and more. Sources are never copied or changed.</p>
              </>
            )}
            <div className="mt-4 flex gap-2">
              <Button size="sm" onClick={() => void pickFolder("Choose a season or movie folder").then((p) => p && addSources([p]))}>
                <FolderPlus /> Add folder…
              </Button>
              <Button size="sm" variant="ghost" onClick={() => void pickVideoFiles().then(addSources)}>
                <FilePlus2 /> Add files…
              </Button>
            </div>
          </div>
        ) : (
          <>
            <div className="max-h-[300px] overflow-y-auto">
              <table className="w-full text-sm">
                <tbody>
                  {candidates.map((c) => {
                    const off = removed.has(c.path)
                    return (
                      <tr key={c.path} className={cn("group border-b border-hairline last:border-0", off && "opacity-40")}>
                        <td className="w-20 py-1.5 pl-3">
                          <span className={cn("timecode text-xs", c.label ? "text-fg-1" : "text-fg-3")}>
                            {c.label || "—"}
                          </span>
                        </td>
                        <td className="max-w-0 py-1.5">
                          <div className="truncate text-fg-2" title={c.path}>
                            {c.fileName}
                          </div>
                        </td>
                        <td className="timecode w-20 py-1.5 pr-2 text-right text-xs text-fg-3">
                          {formatBytes(c.sizeBytes ?? 0)}
                        </td>
                        <td className="w-9 py-1 pr-1.5">
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
                            className="flex size-6 items-center justify-center rounded-sm text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-fg-1 focus-visible:opacity-100"
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
            <div className="flex items-center justify-between border-t border-hairline bg-panel px-3 py-2">
              <span className="text-xs text-fg-3">
                {chosen.length} {chosen.length === 1 ? "video" : "videos"} · {formatBytes(totalBytes)}
                {scanning && <Loader2 className="ml-2 inline size-3 animate-spin" />}
              </span>
              <div className="flex gap-1">
                <Button size="sm" variant="ghost" onClick={() => void pickFolder().then((p) => p && addSources([p]))}>
                  <FolderPlus /> Folder…
                </Button>
                <Button size="sm" variant="ghost" onClick={() => void pickVideoFiles().then(addSources)}>
                  <FilePlus2 /> Files…
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
          {creating && <Loader2 className="animate-spin" />}
          Create project
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
