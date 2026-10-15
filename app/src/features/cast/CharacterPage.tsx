import { useEffect, useMemo, useRef, useState } from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import {
  ArrowLeft,
  ArrowsMerge,
  Check,
  CircleNotch,
  DotsThree,
  Eye,
  EyeSlash,
  Image,
  Package,
  PencilSimple,
  Prohibit,
  User,
  UserFocus,
  UserPlus,
  Users,
  X,
} from "@/components/icons"
import { toast } from "sonner"
import type { CastEdit, Character, CharacterShot, FaceTrackItem, MediaItem } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
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
import { Hero } from "@/components/ui/hero"
import { Input } from "@/components/ui/input"
import { Page } from "@/components/ui/page"
import { Segmented } from "@/components/ui/segmented"
import { TopBar } from "@/components/ui/topbar"
import { ExportDialog, type ExportClip } from "@/features/export/ExportDialog"
import { ShotCard } from "@/features/shots/ShotCard"
import { formatShotLength } from "@/features/shots/shots"
import { useAmbient } from "@/lib/ambient"
import { commands, errorMessage } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { useCast, useCastTracks, useCharacterShots, useMediaList, useProject } from "@/lib/queries"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { castEdit, formatScreenTime, plural } from "./cast"
import { FaceGrid } from "./FaceGrid"

const NO_TRACKS: FaceTrackItem[] = []
const NO_SHOTS: CharacterShot[] = []

type View = "shots" | "faces"
/** The view last chosen, for the next character opened. */
let lastView: View = "shots"

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
  const [chosenView, setView] = useState<View>(lastView)
  // Everyone else has faces, not shots of their own.
  const view: View = everyone ? "faces" : chosenView
  const chooseView = (v: View) => {
    lastView = v
    setView(v)
    setSelected(new Set())
  }
  const mediaById = useMemo(() => new Map((media ?? []).map((m) => [m.id, m])), [media])

  const list = tracks.data ?? NO_TRACKS
  // Tracks that moved away (an edit, a new analysis) drop out of the selection.
  const selected = useMemo(() => {
    const ids = new Set(list.map((t) => t.id))
    return new Set([...picked].filter((id) => ids.has(id)))
  }, [list, picked])

  // Esc clears the selection; Ctrl/⌘+A selects every face (where faces are shown).
  useEffect(() => {
    if (view !== "faces") return
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
  }, [list, selected, view])

  const open = (t: { mediaId: string; start: number }) =>
    void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: t.mediaId }, search: { f: t.start } })

  const toolbar = (
    <TopBar overlay back={{ label: "Cast", to: `/p/${pid}/cast` }} title={everyone ? "Everyone else" : (character?.label ?? " ")} />
  )

  if (!everyone && !isPending && !character) {
    return (
      <Page topbar={toolbar}>
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
          className="mt-24"
        />
      </Page>
    )
  }

  const ids = [...selected]
  const edit = async (e: CastEdit) => {
    if ((await castEdit(pid, e)) !== undefined) setSelected(new Set())
  }

  return (
    <Page topbar={toolbar}>
      <Header pid={pid} character={character} everyone={everyone} faces={list.length} onMerge={() => setMergeOpen(true)} />

      {!everyone && (maybe.data?.length ?? 0) > 0 && character && (
        <Maybe pid={pid} character={character} tracks={maybe.data!} media={mediaById} onOpen={open} />
      )}

      <section className="px-10 pt-2 pb-28">
        <div className="mb-5 flex items-center justify-between gap-4">
          {everyone ? (
            <h2 className="text-xl font-semibold text-fg-1">Faces nobody claimed</h2>
          ) : (
            <Segmented<View>
              aria-label="Show"
              value={view}
              onChange={chooseView}
              options={[
                { value: "shots", label: "Shots" },
                { value: "faces", label: "Faces" },
              ]}
            />
          )}
          {view === "faces" && list.length > 0 && (
            <span className="text-sm text-fg-3">Click to select · Shift for a range · double-click to watch</span>
          )}
        </div>
        {view === "shots" && character ? (
          <Shots pid={pid} character={character} media={mediaById} onOpen={open} />
        ) : tracks.isPending ? (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-3">
            {Array.from({ length: 24 }, (_, i) => (
              <div key={i} className="skeleton aspect-square rounded-xl" />
            ))}
          </div>
        ) : list.length === 0 ? (
          <p className="text-md text-fg-3">No faces here.</p>
        ) : (
          <FaceGrid tracks={list} media={mediaById} selected={selected} onSelect={setSelected} onOpen={open} />
        )}
      </section>

      {view === "faces" && selected.size > 0 && (
        <div className="glass-strong fixed bottom-6 left-1/2 z-40 flex -translate-x-1/2 animate-pop-in items-center gap-1 rounded-full py-1.5 pr-1.5 pl-5">
          <span className="pr-2 text-md font-semibold text-fg-1">{plural(selected.size, "face")}</span>
          <Button size="sm" variant="ghost" onClick={() => setMoveOpen(true)}>
            <UserPlus /> Move to…
          </Button>
          {character && (
            <Button size="sm" variant="ghost" onClick={() => void edit({ op: "not_them", tracks: ids, characterId: character.id })}>
              <Prohibit /> Not {character.label}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => void edit({ op: "not_a_face", tracks: ids })}>
            <X /> Not a face
          </Button>
          {character && selected.size === 1 && (
            <Button size="sm" variant="ghost" onClick={() => void edit({ op: "set_cover", characterId: character.id, track: ids[0] })}>
              <Image /> Use as picture
            </Button>
          )}
          <span className="mx-1 h-5 w-px bg-white/15" />
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

/** The character's shots — what a scenepack of them is made of. */
function Shots({
  pid,
  character,
  media,
  onOpen,
}: {
  pid: string
  character: Character
  media: Map<string, MediaItem>
  onOpen: (s: { mediaId: string; start: number }) => void
}) {
  const shots = useCharacterShots(pid, character.id)
  const list = shots.data ?? NO_SHOTS
  const seconds = useMemo(
    () =>
      list.reduce((sum, s) => {
        const fps = media.get(s.mediaId)?.fps
        return sum + (fps ? ((s.end - s.start) * fps.den) / fps.num : 0)
      }, 0),
    [list, media],
  )
  if (shots.isPending) {
    return (
      <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4">
        {Array.from({ length: 12 }, (_, i) => (
          <div key={i} className="skeleton aspect-video rounded-xl" />
        ))}
      </div>
    )
  }
  if (list.length === 0) {
    return (
      <p className="text-md text-fg-3">
        {character.waiting > 0 ? "Their episode is being analysed again; the shots come back when it's done." : "No shots yet."}
      </p>
    )
  }
  return (
    <>
      <p className="mb-4 text-sm text-fg-3">
        {plural(list.length, "shot")} · {formatShotLength(seconds)} in all · hover to scrub, click to watch
      </p>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4">
        {list.map((s) => (
          <ShotCard key={`${s.mediaId}:${s.start}`} shot={s} media={media.get(s.mediaId)} onOpen={onOpen} />
        ))}
      </div>
    </>
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
  const ambient = useAmbient(c?.cover)

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
    <Hero path={c?.cover} ambient={ambient} blur className="min-h-[clamp(320px,44vh,460px)]">
      <div className="flex flex-wrap items-end justify-between gap-8">
        <div className="flex min-w-0 items-end gap-7">
          <Artwork
            path={c?.cover}
            className="size-40 shrink-0 rounded-[28px] shadow-[0_30px_60px_-20px_rgb(0_0_0/0.8),inset_0_0_0_1px_rgb(255_255_255/0.12)]"
            fallback={
              <div className="flex size-full items-center justify-center bg-white/[0.06] text-white/60 [&_svg]:size-14">
                <User />
              </div>
            }
          />
          <div className="min-w-0 pb-1">
            {everyone ? (
              <h1 className="display text-6xl leading-none text-white">Everyone else</h1>
            ) : editing && c ? (
              <NameField character={c} pid={pid} onDone={() => setEditing(false)} />
            ) : (
              <button
                onClick={() => setEditing(true)}
                className="group -mx-2 flex max-w-full items-center gap-3 rounded-xl px-2 text-left transition-colors hover:bg-white/[0.06]"
                title="Rename"
              >
                <h1 className={cn("display truncate text-6xl leading-[1.05] text-white", !c?.name && "text-white/75 italic")}>
                  {c?.label ?? " "}
                </h1>
                <PencilSimple className="size-5 shrink-0 text-white/60 opacity-0 transition-opacity group-hover:opacity-100" />
              </button>
            )}
            <div className="mt-4 flex flex-wrap items-center gap-2">
              {everyone ? (
                <>
                  <Badge tone="media">{plural(faces, "face")}</Badge>
                  <span className="text-sm text-white/70">Extras, background faces and anyone seen too briefly to tell</span>
                </>
              ) : c ? (
                <>
                  <Badge tone="media">{formatScreenTime(c.screenSec ?? 0)} on screen</Badge>
                  <Badge tone="media">{plural(c.shots, "shot")}</Badge>
                  <Badge tone="media">{plural(c.media, "episode")}</Badge>
                  <Badge tone="media">{plural(c.tracks, "face")}</Badge>
                  {!c.name && <span className="pl-1 text-sm text-white/60">Click the name to rename</span>}
                </>
              ) : null}
            </div>
          </div>
        </div>
        {c && (
          <div className="flex shrink-0 items-center gap-2.5 pb-1">
            <Button size="lg" variant="primary" disabled={loading || c.shots === 0} onClick={() => void startExport()}>
              {loading ? <CircleNotch className="animate-spin" /> : <Package weight="bold" />} Export {plural(c.shots, "shot")}
            </Button>
            <Button size="lg" variant="glass" onClick={onMerge}>
              <ArrowsMerge /> Same person as…
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="icon-lg" variant="glass" aria-label={`${c.label} options`}>
                  <DotsThree weight="bold" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => setEditing(true)}>
                  <PencilSimple /> Rename
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onMerge}>
                  <ArrowsMerge /> Same person as…
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => void castEdit(pid, { op: "set_hidden", characterId: c.id, hidden: !c.hidden })}
                >
                  {c.hidden ? <Eye /> : <EyeSlash />} {c.hidden ? "Show in the cast" : "Hide from the cast"}
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
    </Hero>
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
      className="display h-16 w-[min(520px,60vw)] rounded-2xl border-white/20 bg-black/30 px-4 text-5xl backdrop-blur-md"
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
    <section className="mx-10 mb-10 rounded-3xl border border-accent-line/60 bg-[linear-gradient(135deg,rgb(224_180_92/0.1),rgb(255_255_255/0.02)_60%)] p-6">
      <div className="mb-5 flex flex-wrap items-center justify-between gap-4">
        <div>
          <h2 className="display text-3xl text-fg-1">Maybe {c.label}?</h2>
          <p className="mt-1 text-sm text-fg-2">
            {plural(tracks.length, "face")} {tracks.length === 1 ? "looks" : "look"} like {c.label}, but not clearly enough to
            decide alone.
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <Button variant="secondary" asChild>
            <Link to="/p/$pid/cast/review" params={{ pid }} search={{ c: c.id }}>
              <UserFocus /> Review one by one
            </Link>
          </Button>
          <Button variant="accent" onClick={() => void castEdit(pid, { op: "confirm", tracks: tracks.map((t) => t.id) })}>
            <Check weight="bold" /> All are {c.label}
          </Button>
        </div>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-3">
        {shown.map((t) => (
          <div key={t.id} className="group relative aspect-square overflow-hidden rounded-xl bg-inset picture-edge">
            <img
              src={assetUrl(t.thumb)}
              alt=""
              loading="lazy"
              className="size-full object-cover"
              onDoubleClick={() => onOpen(t)}
              title={media.get(t.mediaId)?.label}
            />
            <div className="absolute inset-x-1 bottom-1 flex justify-between gap-1 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <button
                onClick={() => void castEdit(pid, { op: "confirm", tracks: [t.id] })}
                aria-label={`Yes, ${c.label}`}
                className="flex h-7 flex-1 items-center justify-center rounded-lg bg-black/60 text-white backdrop-blur-md hover:bg-accent hover:text-on-accent"
              >
                <Check className="size-3.5" />
              </button>
              <button
                onClick={() => void castEdit(pid, { op: "not_them", tracks: [t.id], characterId: c.id })}
                aria-label={`Not ${c.label}`}
                className="flex h-7 flex-1 items-center justify-center rounded-lg bg-black/60 text-white backdrop-blur-md hover:bg-danger hover:text-white"
              >
                <X className="size-3.5" />
              </button>
            </div>
          </div>
        ))}
      </div>
      {tracks.length > 12 && (
        <button onClick={() => setAll((v) => !v)} className="mt-4 text-sm font-medium text-fg-2 hover:text-fg-1">
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
              <span className="size-14 shrink-0 overflow-hidden rounded-full bg-inset picture-edge">
                {pending.cover && <img src={assetUrl(pending.cover)} alt="" className="size-full object-cover" />}
              </span>
              <div>
                <p className="text-md font-medium text-fg-1">{asked.question}</p>
                <p className="mt-1 text-sm text-fg-3">{asked.detail}</p>
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
            <div id="pick-character-list" role="listbox" className="-mx-2 mt-3 max-h-[360px] overflow-y-auto px-1">
              {allowNew && (
                <button
                  onClick={() => onPick({ kind: "new", name: query.trim() })}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left text-md text-fg-1 hover:bg-hover",
                    selected == null && q && "bg-hover",
                  )}
                >
                  <span className="flex size-10 items-center justify-center rounded-full border border-dashed border-white/25 text-fg-3">
                    <UserPlus className="size-4" />
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
                    "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left hover:bg-hover",
                    i === selected && "bg-active",
                  )}
                >
                  <span className="size-10 shrink-0 overflow-hidden rounded-full bg-inset picture-edge">
                    {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className={cn("block truncate text-md font-medium", c.name ? "text-fg-1" : "text-fg-2")}>{c.label}</span>
                    <span className="block text-xs text-fg-3">
                      {formatScreenTime(c.screenSec ?? 0)} · {plural(c.shots, "shot")}
                    </span>
                  </span>
                </button>
              ))}
              {!allowNew && options.length === 0 && <p className="px-3 py-4 text-md text-fg-3">Nobody matches.</p>}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
