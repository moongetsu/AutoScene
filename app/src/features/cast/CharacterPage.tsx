import { useEffect, useMemo, useRef, useState } from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import {
  ArrowLeft,
  Ban,
  Check,
  Eye,
  EyeOff,
  ImageIcon,
  Loader2,
  Merge,
  MoreHorizontal,
  PackageOpen,
  Pencil,
  ScanFace,
  UserRound,
  UserRoundPlus,
  Users,
  X,
} from "lucide-react"
import { toast } from "sonner"
import type { CastEdit, Character, FaceTrackItem, MediaItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Input } from "@/components/ui/input"
import { Page } from "@/components/ui/page"
import { ExportDialog, type ExportClip } from "@/features/export/ExportDialog"
import { commands, errorMessage } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { useCast, useCastTracks, useMediaList, useProject } from "@/lib/queries"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { castEdit, formatScreenTime, plural } from "./cast"
import { FaceGrid } from "./FaceGrid"

const NO_TRACKS: FaceTrackItem[] = []

/** One character (or everyone nobody claimed): faces, decisions, export. */
export function CharacterPage() {
  const { pid, cid } = useParams({ from: "/p/$pid/cast/$cid" })
  const navigate = useNavigate()
  const { data: cast, isPending } = useCast(pid)
  const { data: media } = useMediaList(pid)
  const everyone = cid === "unassigned"
  const character = cast?.characters.find((c) => c.id === cid)
  const tracks = useCastTracks(pid, everyone ? { kind: "unassigned" } : { kind: "character", id: cid })
  const maybe = useCastTracks(pid, { kind: "unsure", id: cid }, !everyone)
  const [picked, setSelected] = useState<Set<number>>(new Set())
  const [moveOpen, setMoveOpen] = useState(false)
  const [mergeOpen, setMergeOpen] = useState(false)
  const mediaById = useMemo(() => new Map((media ?? []).map((m) => [m.id, m])), [media])

  const list = tracks.data ?? NO_TRACKS
  // Tracks that moved away (an edit, a new analysis) drop out of the selection.
  const selected = useMemo(() => {
    const ids = new Set(list.map((t) => t.id))
    return new Set([...picked].filter((id) => ids.has(id)))
  }, [list, picked])

  // Esc clears the selection; Ctrl/⌘+A selects every face.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof Element && e.target.closest("input, textarea, [role=dialog], [role=menu]")) return
      if (e.key === "Escape" && selected.size > 0) setSelected(new Set())
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault()
        setSelected(new Set(list.map((t) => t.id)))
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [list, selected])

  const open = (t: FaceTrackItem) =>
    void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: t.mediaId }, search: { f: t.start } })

  if (!everyone && !isPending && !character) {
    return (
      <Page>
        <EmptyState
          icon={<Users />}
          title="This character isn't in the cast any more"
          description="It may have been merged into someone else."
          actions={
            <Button asChild>
              <Link to="/p/$pid/cast" params={{ pid }}>
                <ArrowLeft /> Back to the cast
              </Link>
            </Button>
          }
          className="mt-16"
        />
      </Page>
    )
  }

  const ids = [...selected]
  const edit = async (e: CastEdit) => {
    if ((await castEdit(pid, e)) !== undefined) setSelected(new Set())
  }

  return (
    <Page>
      <Header pid={pid} character={character} everyone={everyone} faces={list.length} onMerge={() => setMergeOpen(true)} />

      {!everyone && (maybe.data?.length ?? 0) > 0 && character && (
        <Maybe pid={pid} character={character} tracks={maybe.data!} media={mediaById} onOpen={open} />
      )}

      <section className="px-8 pb-24">
        <div className="mb-3 flex items-baseline justify-between">
          <h2 className="eyebrow">{everyone ? "Faces nobody claimed" : "Faces"}</h2>
          {list.length > 0 && (
            <span className="text-2xs text-fg-3">Click to select · Shift for a range · double-click to watch</span>
          )}
        </div>
        {tracks.isPending ? (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(96px,1fr))] gap-2">
            {Array.from({ length: 24 }, (_, i) => (
              <div key={i} className="skeleton aspect-square rounded-md" />
            ))}
          </div>
        ) : list.length === 0 ? (
          <p className="text-sm text-fg-3">No faces here.</p>
        ) : (
          <FaceGrid tracks={list} media={mediaById} selected={selected} onSelect={setSelected} onOpen={open} />
        )}
      </section>

      {selected.size > 0 && (
        <div className="glass fixed bottom-5 left-1/2 z-40 flex -translate-x-1/2 items-center gap-1 rounded-xl px-2 py-1.5 shadow-2">
          <span className="px-2 text-sm text-fg-1">{plural(selected.size, "face")}</span>
          <Button size="sm" variant="ghost" onClick={() => setMoveOpen(true)}>
            <UserRoundPlus /> Move to…
          </Button>
          {character && (
            <Button size="sm" variant="ghost" onClick={() => void edit({ op: "not_them", tracks: ids, characterId: character.id })}>
              <Ban /> Not {character.label}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => void edit({ op: "not_a_face", tracks: ids })}>
            <X /> Not a face
          </Button>
          {character && selected.size === 1 && (
            <Button size="sm" variant="ghost" onClick={() => void edit({ op: "set_cover", characterId: character.id, track: ids[0] })}>
              <ImageIcon /> Use as picture
            </Button>
          )}
          <span className="mx-1 h-5 w-px bg-hairline" />
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())} aria-label="Clear selection">
            Clear
          </Button>
        </div>
      )}

      <PickCharacter
        pid={pid}
        open={moveOpen}
        onOpenChange={setMoveOpen}
        title={`Move ${plural(selected.size, "face")} to…`}
        exclude={character?.id}
        allowNew
        onPick={async (target) => {
          setMoveOpen(false)
          if (target.kind === "new") await edit({ op: "new_character", tracks: ids, name: target.name || null })
          else await edit({ op: "assign", tracks: ids, characterId: target.id })
        }}
      />
      {character && (
        <PickCharacter
          pid={pid}
          open={mergeOpen}
          onOpenChange={setMergeOpen}
          title={`${character.label} is the same person as…`}
          description="Every face of both becomes one character. It keeps a name if either has one."
          exclude={character.id}
          confirm={(target) => ({
            question: `Merge ${character.label} into ${target.label}?`,
            detail: "This can't be undone. Faces you said aren't them stay apart.",
            action: "Merge",
          })}
          onPick={async (target) => {
            setMergeOpen(false)
            if (target.kind !== "existing") return
            if ((await castEdit(pid, { op: "merge", from: character.id, into: target.id })) !== undefined) {
              void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: target.id }, replace: true })
            }
          }}
        />
      )}
    </Page>
  )
}

function Header({
  pid,
  character: c,
  everyone,
  faces,
  onMerge,
}: {
  pid: string
  character?: Character
  everyone: boolean
  faces: number
  onMerge: () => void
}) {
  const [editing, setEditing] = useState(false)
  const [exportClips, setExportClips] = useState<ExportClip[] | null>(null)
  const [loading, setLoading] = useState(false)
  const { data: project } = useProject(pid)
  const { data: media } = useMediaList(pid)

  const startExport = async () => {
    if (!c) return
    setLoading(true)
    try {
      const clips = await commands.characterClips(pid, c.id)
      const byId = new Map((media ?? []).map((m) => [m.id, m]))
      setExportClips(clips.map((clip) => toExportClip(clip, byId.get(clip.mediaId), project?.title ?? "", c.label)))
    } catch (e) {
      toast.error("Couldn't list the shots", { description: errorMessage(e) })
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="px-8 pt-6 pb-6">
      <Link to="/p/$pid/cast" params={{ pid }} className="inline-flex items-center gap-1 text-xs text-fg-3 hover:text-fg-1">
        <ArrowLeft className="size-3.5" /> Cast
      </Link>
      <div className="mt-3 flex items-end justify-between gap-6">
        <div className="flex min-w-0 items-center gap-4">
          <div className="size-16 shrink-0 overflow-hidden rounded-lg bg-inset shadow-1">
            {c?.cover ? (
              <img src={assetUrl(c.cover)} alt="" className="size-full object-cover" />
            ) : (
              <UserRound className="m-auto mt-4 size-8 text-fg-3" />
            )}
          </div>
          <div className="min-w-0">
            {everyone ? (
              <h1 className="text-2xl text-fg-1">Everyone else</h1>
            ) : editing && c ? (
              <NameField character={c} pid={pid} onDone={() => setEditing(false)} />
            ) : (
              <button
                onClick={() => setEditing(true)}
                className="group -mx-1 flex max-w-full items-center gap-2 rounded-md px-1 text-left hover:bg-hover"
                title="Rename"
              >
                <h1 className={cn("truncate text-2xl", c?.name ? "text-fg-1" : "text-fg-2")}>{c?.label ?? " "}</h1>
                <Pencil className="size-3.5 shrink-0 text-fg-3 opacity-0 transition-opacity group-hover:opacity-100" />
              </button>
            )}
            <p className="mt-1 text-sm text-fg-2">
              {everyone ? (
                <>Extras, background faces and anyone seen too briefly to tell · {plural(faces, "face")}</>
              ) : c ? (
                <>
                  {formatScreenTime(c.screenSec ?? 0)} on screen · {plural(c.shots, "shot")} · {plural(c.media, "episode")} ·{" "}
                  {plural(c.tracks, "face")}
                  {!c.name && <span className="text-fg-3"> · click the name to rename</span>}
                </>
              ) : null}
            </p>
          </div>
        </div>
        {c && (
          <div className="flex shrink-0 items-center gap-2">
            <Button variant="primary" disabled={loading || c.shots === 0} onClick={() => void startExport()}>
              {loading ? <Loader2 className="animate-spin" /> : <PackageOpen />} Export {plural(c.shots, "shot")}
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="icon" variant="ghost" aria-label={`${c.label} options`}>
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => setEditing(true)}>
                  <Pencil /> Rename
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onMerge}>
                  <Merge /> Same person as…
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => void castEdit(pid, { op: "set_hidden", characterId: c.id, hidden: !c.hidden })}
                >
                  {c.hidden ? <Eye /> : <EyeOff />} {c.hidden ? "Show in the cast" : "Hide from the cast"}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        )}
      </div>
      {exportClips && (
        <ExportDialog
          projectId={pid}
          clips={exportClips}
          open={exportClips.length > 0}
          onOpenChange={(o) => !o && setExportClips(null)}
        />
      )}
    </div>
  )
}

/** `Show_Character_S01E03_00-12-03-05`, the clip's file name in a character pack. */
function toExportClip(
  clip: { mediaId: string; start: number; end: number },
  m: MediaItem | undefined,
  title: string,
  who: string,
): ExportClip {
  const tc = m?.fps ? timecode(clip.start, m.fps, false) : String(clip.start)
  const what = m ? m.label || m.fileName.replace(/\.[^.]+$/, "") : ""
  const name = [title, who, what, tc.replace(/[:;]/g, "-")]
    .filter(Boolean)
    .join(" ")
    .replace(/[<>:"/\\|?*]+/g, " ")
    .trim()
    .replace(/\s+/g, "_")
  return {
    mediaId: clip.mediaId,
    start: clip.start,
    end: clip.end,
    name,
    summary: `${m?.label ? `${m.label} · ` : ""}${tc} · ${clip.end - clip.start} frames`,
    suggestedName: name,
  }
}

function NameField({ character: c, pid, onDone }: { character: Character; pid: string; onDone: () => void }) {
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => {
    ref.current?.focus()
    ref.current?.select()
  }, [])
  const save = async () => {
    const name = ref.current?.value.trim() ?? ""
    if (name !== (c.name ?? "")) await castEdit(pid, { op: "rename", characterId: c.id, name: name || null })
    onDone()
  }
  return (
    <Input
      ref={ref}
      defaultValue={c.name ?? ""}
      placeholder={c.label}
      aria-label="Character name"
      className="h-10 w-[min(360px,60vw)] text-xl"
      onBlur={() => void save()}
      onKeyDown={(e) => {
        if (e.key === "Enter") void save()
        if (e.key === "Escape") onDone()
      }}
    />
  )
}

/** Faces that look like this character but need a yes or no. */
function Maybe({
  pid,
  character: c,
  tracks,
  media,
  onOpen,
}: {
  pid: string
  character: Character
  tracks: FaceTrackItem[]
  media: Map<string, MediaItem>
  onOpen: (t: FaceTrackItem) => void
}) {
  const [all, setAll] = useState(false)
  const shown = all ? tracks : tracks.slice(0, 12)
  return (
    <section className="mx-8 mb-8 rounded-lg border border-hairline bg-panel p-4 shadow-1">
      <div className="mb-3 flex items-center justify-between gap-4">
        <div>
          <h2 className="text-sm font-medium text-fg-1">Maybe {c.label}?</h2>
          <p className="text-2xs text-fg-3">
            {plural(tracks.length, "face")} {tracks.length === 1 ? "looks" : "look"} like {c.label}, but not clearly enough to
            decide alone.
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <Button size="sm" variant="ghost" asChild>
            <Link to="/p/$pid/cast/review" params={{ pid }} search={{ c: c.id }}>
              <ScanFace /> Review one by one
            </Link>
          </Button>
          <Button size="sm" onClick={() => void castEdit(pid, { op: "confirm", tracks: tracks.map((t) => t.id) })}>
            <Check /> All are {c.label}
          </Button>
        </div>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(88px,1fr))] gap-2">
        {shown.map((t) => (
          <div key={t.id} className="group relative aspect-square overflow-hidden rounded-md bg-inset">
            <img
              src={assetUrl(t.thumb)}
              alt=""
              loading="lazy"
              className="size-full cursor-pointer object-cover"
              onDoubleClick={() => onOpen(t)}
              title={media.get(t.mediaId)?.label}
            />
            <div className="absolute inset-x-1 bottom-1 flex justify-between gap-1 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <button
                onClick={() => void castEdit(pid, { op: "confirm", tracks: [t.id] })}
                aria-label={`Yes, ${c.label}`}
                className="flex h-6 flex-1 items-center justify-center rounded-sm bg-canvas/85 text-fg-1 hover:bg-accent hover:text-on-accent"
              >
                <Check className="size-3.5" />
              </button>
              <button
                onClick={() => void castEdit(pid, { op: "not_them", tracks: [t.id], characterId: c.id })}
                aria-label={`Not ${c.label}`}
                className="flex h-6 flex-1 items-center justify-center rounded-sm bg-canvas/85 text-fg-1 hover:bg-danger hover:text-on-accent"
              >
                <X className="size-3.5" />
              </button>
            </div>
          </div>
        ))}
      </div>
      {tracks.length > 12 && (
        <button onClick={() => setAll((v) => !v)} className="mt-2 text-2xs text-fg-3 hover:text-fg-1">
          {all ? "Show fewer" : `Show ${plural(tracks.length - shown.length, "more face")}`}
        </button>
      )}
    </section>
  )
}

type Pick = { kind: "existing"; id: string } | { kind: "new"; name: string }

/** Choose a character (or start a new one), optionally confirming the choice. */
export function PickCharacter({
  pid,
  open,
  onOpenChange,
  title,
  description,
  exclude,
  allowNew,
  confirm,
  onPick,
}: {
  pid: string
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  exclude?: string
  allowNew?: boolean
  /** Ask before an existing character is taken (for what can't be undone). */
  confirm?: (target: Character) => { question: string; detail: string; action: string }
  onPick: (p: Pick) => void
}) {
  const { data: cast } = useCast(pid)
  const [query, setQuery] = useState("")
  // The row arrows moved to (an index into `options`); null: none yet.
  const [active, setActive] = useState<number | null>(null)
  const [pending, setPending] = useState<Character | null>(null)
  const q = query.trim().toLowerCase()
  const options = (cast?.characters ?? []).filter((c) => c.id !== exclude && (!q || c.label.toLowerCase().includes(q)))
  // Enter takes the row the arrows chose, else an exact name — never just
  // the first partial match (typing a new name must not pick an old one).
  const exact = options.findIndex((c) => c.label.toLowerCase() === q)
  const selected = active ?? (exact >= 0 ? exact : null)
  const reset = () => {
    setQuery("")
    setActive(null)
    setPending(null)
  }
  const choose = (c: Character) => (confirm ? setPending(c) : onPick({ kind: "existing", id: c.id }))
  const asked = pending && confirm?.(pending)
  return (
    <Dialog open={open} onOpenChange={(o) => (onOpenChange(o), reset())}>
      <DialogContent title={title} description={description} className="w-[min(440px,calc(100vw-32px))]">
        {pending && asked ? (
          <div className="flex flex-col gap-4">
            <div className="flex items-center gap-3">
              <span className="size-12 shrink-0 overflow-hidden rounded-md bg-inset">
                {pending.cover && <img src={assetUrl(pending.cover)} alt="" className="size-full object-cover" />}
              </span>
              <div>
                <p className="text-sm text-fg-1">{asked.question}</p>
                <p className="mt-0.5 text-2xs text-fg-3">{asked.detail}</p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setPending(null)}>
                Back
              </Button>
              <Button variant="primary" autoFocus onClick={() => onPick({ kind: "existing", id: pending.id })}>
                {asked.action}
              </Button>
            </div>
          </div>
        ) : (
          <>
            <Input
              autoFocus
              placeholder={allowNew ? "Search, or type a new name" : "Search"}
              value={query}
              role="combobox"
              aria-expanded
              aria-controls="pick-character-list"
              aria-activedescendant={selected != null && options[selected] ? `pick-${options[selected].id}` : undefined}
              onChange={(e) => {
                setQuery(e.target.value)
                setActive(null)
              }}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault()
                  if (options.length === 0) return
                  const from = selected ?? (e.key === "ArrowDown" ? -1 : options.length)
                  setActive(Math.min(options.length - 1, Math.max(0, from + (e.key === "ArrowDown" ? 1 : -1))))
                } else if (e.key === "Enter") {
                  e.preventDefault()
                  if (selected != null && options[selected]) choose(options[selected])
                  else if (allowNew && q) onPick({ kind: "new", name: query.trim() })
                }
              }}
            />
            <div id="pick-character-list" role="listbox" className="mt-3 max-h-[320px] overflow-y-auto">
              {allowNew && (
                <button
                  onClick={() => onPick({ kind: "new", name: query.trim() })}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm text-fg-1 hover:bg-hover",
                    selected == null && q && "bg-hover",
                  )}
                >
                  <span className="flex size-8 items-center justify-center rounded-md border border-dashed border-soft text-fg-3">
                    <UserRoundPlus className="size-4" />
                  </span>
                  {query.trim() ? <>New character “{query.trim()}”</> : "New character"}
                </button>
              )}
              {options.map((c, i) => (
                <button
                  key={c.id}
                  id={`pick-${c.id}`}
                  role="option"
                  aria-selected={i === selected}
                  onClick={() => choose(c)}
                  onMouseMove={() => active !== i && setActive(i)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left hover:bg-hover",
                    i === selected && "bg-hover ring-1 ring-strong",
                  )}
                >
                  <span className="size-8 shrink-0 overflow-hidden rounded-md bg-inset">
                    {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className={cn("block truncate text-sm", c.name ? "text-fg-1" : "text-fg-2")}>{c.label}</span>
                    <span className="block text-2xs text-fg-3">
                      {formatScreenTime(c.screenSec ?? 0)} · {plural(c.shots, "shot")}
                    </span>
                  </span>
                </button>
              ))}
              {!allowNew && options.length === 0 && <p className="px-2 py-3 text-sm text-fg-3">Nobody matches.</p>}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
