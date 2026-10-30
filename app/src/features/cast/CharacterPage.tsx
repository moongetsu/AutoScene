import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { AnimatePresence, motion } from "motion/react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import {
  ArrowLeft,
  ArrowsMerge,
  Check,
  DotsThree,
  FilmStrip,
  Image,
  Package,
  PencilSimple,
  Play,
  Prohibit,
  SelectionAll,
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
import { ContextArea } from "@/components/ui/context-area"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { ErrorState } from "@/components/ui/error-state"
import { Hero } from "@/components/ui/hero"
import { Input } from "@/components/ui/input"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { contextKit, dropdownKit } from "@/components/ui/menu-actions"
import { Page } from "@/components/ui/page"
import { Segmented } from "@/components/ui/segmented"
import { Tooltip } from "@/components/ui/tooltip"
import { TopBar } from "@/components/ui/topbar"
import { VirtualGrid } from "@/components/ui/virtual-grid"
import { ProjectLoadError } from "@/features/project/ProjectLoadError"
import { ShotCard } from "@/features/shots/ShotCard"
import { type BodyVerdict, formatShotLength } from "@/features/shots/shots"
import { useAmbient } from "@/lib/ambient"
import { useFittedTitle } from "@/lib/fit-text"
import { assetUrl } from "@/lib/media"
import { modKey } from "@/lib/platform"
import { firstFailure, useCast, useCastTracks, useCharacterShots, useMediaList } from "@/lib/queries"
import { FACES_SHORTCUTS, useShortcuts } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { castEdit, formatScreenTime, plural } from "./cast"
import { CharacterItems } from "./character-actions"
import { FaceGrid } from "./FaceGrid"
import { PickCharacter } from "./PickCharacter"

const NO_TRACKS: FaceTrackItem[] = []
const NO_SHOTS: CharacterShot[] = []

type View = "shots" | "faces"
/** The view last chosen, for the next character opened. */
let lastView: View = "shots"

/** One character (or everyone nobody claimed): faces, decisions, export. */
export function CharacterPage() {
  const { pid, cid } = useParams({ from: "/p/$pid/cast/$cid" })
  const navigate = useNavigate()
  const castQuery = useCast(pid)
  const mediaQuery = useMediaList(pid)
  const { data: cast } = castQuery
  const { data: media } = mediaQuery
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
  useShortcuts(
    FACES_SHORTCUTS,
    {
      selectAll: () => setSelected(new Set(list.map((t) => t.id))),
      clearSelection: () => {
        if (selected.size === 0) return false
        setSelected(new Set())
      },
    },
    { enabled: view === "faces" },
  )

  const open = useCallback(
    (t: { mediaId: string; start: number }) =>
      void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: t.mediaId }, search: { f: t.start } }),
    [navigate, pid],
  )

  const toolbar = (
    <TopBar overlay back={{ label: "Cast", to: `/p/${pid}/cast` }} title={everyone ? "Everyone else" : (character?.label ?? " ")} />
  )

  const failure = firstFailure(castQuery, mediaQuery, tracks)
  if (failure) {
    return (
      <Page topbar={toolbar}>
        <ProjectLoadError error={failure.error} onRetry={failure.retry} />
      </Page>
    )
  }

  // Only a cast that loaded can say someone left it.
  if (!everyone && cast && !character) {
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

  // What the faces selected can be told (the bar at the bottom says the same).
  const selectionMenu = selected.size > 0 && (
    <>
      <ContextMenuItem onSelect={() => setMoveOpen(true)}>
        <UserPlus /> Move {plural(selected.size, "face")} to…
      </ContextMenuItem>
      {character && (
        <ContextMenuItem onSelect={() => void edit({ op: "not_them", tracks: ids, characterId: character.id })}>
          <Prohibit /> Not {character.label}
        </ContextMenuItem>
      )}
      <ContextMenuItem onSelect={() => void edit({ op: "not_a_face", tracks: ids })}>
        <X /> Not a face
      </ContextMenuItem>
      {character && selected.size === 1 && (
        <ContextMenuItem onSelect={() => void edit({ op: "set_cover", characterId: character.id, track: ids[0] })}>
          <Image /> Use as picture
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={() => setSelected(new Set(list.map((t) => t.id)))} shortcut={[modKey, "A"]}>
        <SelectionAll /> Select all
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => setSelected(new Set())} shortcut={["Esc"]}>
        <X /> Clear selection
      </ContextMenuItem>
    </>
  )
  const pageMenu = (
    <>
      {character && (
        <CharacterItems kit={contextKit} pid={pid} character={character} open={false} onMerge={() => setMergeOpen(true)} />
      )}
      <ContextMenuSeparator />
      {!everyone && (
        <ContextMenuItem onSelect={() => chooseView(view === "shots" ? "faces" : "shots")}>
          {view === "shots" ? <UserFocus /> : <FilmStrip />} {view === "shots" ? "Show their faces" : "Show their shots"}
        </ContextMenuItem>
      )}
      {view === "faces" && list.length > 0 && (
        <ContextMenuItem onSelect={() => setSelected(new Set(list.map((t) => t.id)))} shortcut={[modKey, "A"]}>
          <SelectionAll /> Select every face
        </ContextMenuItem>
      )}
    </>
  )
  const onBody = (bodies: number[], verdict: BodyVerdict) => {
    if (!character) return
    const e: CastEdit =
      verdict === "not_them"
        ? { op: "body_not_them", bodies, characterId: character.id }
        : { op: "not_a_person", bodies }
    void castEdit(pid, e).then((r) => {
      if (r !== undefined) toast(verdict === "not_them" ? `Not ${character.label}: out of their shots` : "Not a person: out of their shots")
    })
  }

  return (
    <Page topbar={toolbar} menu={pageMenu}>
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
          <Shots pid={pid} character={character} media={mediaById} onOpen={open} onBody={onBody} />
        ) : tracks.isPending ? (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-3">
            {Array.from({ length: 24 }, (_, i) => (
              <div key={i} className="skeleton aspect-square rounded-xl" />
            ))}
          </div>
        ) : list.length === 0 ? (
          <p className="text-md text-fg-3">No faces here.</p>
        ) : (
          <ContextArea menu={selectionMenu}>
            <div>
              <FaceGrid tracks={list} media={mediaById} selected={selected} onSelect={setSelected} onOpen={open} />
            </div>
          </ContextArea>
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
  onBody,
}: {
  pid: string
  character: Character
  media: Map<string, MediaItem>
  onOpen: (s: { mediaId: string; start: number }) => void
  onBody: (bodies: number[], verdict: BodyVerdict) => void
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
  if (shots.isError && shots.data === undefined) {
    return <ErrorState error={shots.error} title="Their shots couldn't be loaded" onRetry={() => void shots.refetch()} />
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
      <VirtualGrid
        items={list}
        minItemWidth={240}
        itemHeight={(w) => (w * 9) / 16}
        getKey={(s) => `${s.mediaId}:${s.start}`}
        renderItem={(s) => (
          <ShotCard shot={s} media={media.get(s.mediaId)} name={character.label} onOpen={onOpen} onBody={onBody} />
        )}
      />
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
  // The backdrop is blurred: the small face does, and stays put when the portrait arrives.
  const ambient = useAmbient(c?.coverSmall)
  const [row, setRow] = useState<HTMLDivElement | null>(null)

  return (
    <ContextArea
      disabled={!c}
      menu={
        c && <CharacterItems kit={contextKit} pid={pid} character={c} open={false} onRename={() => setEditing(true)} onMerge={onMerge} />
      }
    >
      <Hero path={c?.coverSmall} ambient={ambient} blur className="min-h-[clamp(320px,44vh,460px)]">
        <div ref={setRow} className="flex flex-wrap items-end justify-between gap-8">
          <div className="flex min-w-0 items-end gap-7">
            <Artwork
              key={c?.id}
              path={c?.cover}
              className="size-40 shrink-0 rounded-[28px] shadow-[0_30px_60px_-20px_rgb(0_0_0/0.8),inset_0_0_0_1px_rgb(255_255_255/0.12)]"
              imgClassName="object-[50%_30%]"
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
                <Tooltip label="Rename" side="bottom">
                  {/* Its margins reach 1rem past the column it sizes; so may it, or the name is cut 1rem short. */}
                  <button
                    onClick={() => setEditing(true)}
                    className="group -mx-2 flex max-w-[calc(100%+1rem)] items-center gap-3 rounded-xl px-2 text-left transition-colors hover:bg-white/[0.06]"
                  >
                    <HeroName row={row} named={!!c?.name} text={c?.label ?? " "} />
                    <PencilSimple className="size-5 shrink-0 text-white/60 opacity-0 transition-opacity group-hover:opacity-100" />
                  </button>
                </Tooltip>
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
              {c.shots > 0 ? (
                <Button size="lg" variant="primary" asChild>
                  <Link to="/p/$pid/cast/$cid/pack" params={{ pid, cid: c.id }}>
                    <Package weight="bold" /> Build a scenepack
                  </Link>
                </Button>
              ) : (
                <Button size="lg" variant="primary" disabled>
                  <Package weight="bold" /> Build a scenepack
                </Button>
              )}
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
                  <CharacterItems
                    kit={dropdownKit}
                    pid={pid}
                    character={c}
                    open={false}
                    onRename={() => setEditing(true)}
                    onMerge={onMerge}
                  />
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          )}
        </div>
      </Hero>
    </ContextArea>
  )
}

/** The character's name, as big as the hero's row has room for (a long name gets smaller). */
function HeroName({ row, named, text }: { row: HTMLDivElement | null; named: boolean; text: string }) {
  const ref = useRef<HTMLHeadingElement>(null)
  const size = useFittedTitle(ref, row, text, 48)
  return (
    <h1
      ref={ref}
      style={size ? { fontSize: size } : undefined}
      className={cn("display truncate text-6xl leading-[1.05] text-white", !named && "text-white/75 italic")}
    >
      {text}
    </h1>
  )
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
  const [dismissed, setDismissed] = useState<Set<number>>(new Set())
  const activeTracks = useMemo(() => tracks.filter((t) => !dismissed.has(t.id)), [tracks, dismissed])
  const shown = all ? activeTracks : activeTracks.slice(0, 12)
  const navigate = useNavigate()

  const handleAction = async (edit: CastEdit, ids: number[]) => {
    setDismissed((prev) => new Set([...prev, ...ids]))
    const res = await castEdit(pid, edit)
    if (res === undefined) {
      // Revert optimistic dismissal if backend failed
      setDismissed((prev) => {
        const next = new Set(prev)
        for (const id of ids) next.delete(id)
        return next
      })
    }
  }

  if (activeTracks.length === 0) return null

  return (
    <ContextArea
      menu={
        <>
          <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid/cast/review", params: { pid }, search: { c: c.id } })}>
            <UserFocus /> Review one by one
          </ContextMenuItem>
          <ContextMenuItem
            onSelect={() => void handleAction({ op: "assign", tracks: activeTracks.map((t) => t.id), characterId: c.id }, activeTracks.map((t) => t.id))}
          >
            <Check /> All are {c.label}
          </ContextMenuItem>
        </>
      }
    >
      <section className="mx-10 mb-10 rounded-3xl border border-accent-line/60 bg-[linear-gradient(135deg,rgb(224_180_92/0.1),rgb(255_255_255/0.02)_60%)] p-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-4">
          <div>
            <h2 className="display text-3xl text-fg-1">Maybe {c.label}?</h2>
            <p className="mt-1 text-sm text-fg-2">
              {plural(activeTracks.length, "face")} {activeTracks.length === 1 ? "looks" : "look"} like {c.label}, but not clearly enough to
              decide alone.
            </p>
          </div>
          <div className="flex shrink-0 gap-2">
            <Button variant="secondary" asChild>
              <Link to="/p/$pid/cast/review" params={{ pid }} search={{ c: c.id }}>
                <UserFocus /> Review one by one
              </Link>
            </Button>
            <Button
              variant="accent"
              onClick={() => void handleAction({ op: "assign", tracks: activeTracks.map((t) => t.id), characterId: c.id }, activeTracks.map((t) => t.id))}
            >
              <Check weight="bold" /> All are {c.label}
            </Button>
          </div>
        </div>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-3">
          <AnimatePresence mode="popLayout">
            {shown.map((t) => (
              <motion.div
                key={t.id}
                layout
                initial={{ opacity: 0, scale: 0.9 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.8 }}
                transition={{ duration: 0.15, ease: "easeOut" }}
              >
                <ContextArea
                  menu={
                    <>
                      <ContextMenuItem onSelect={() => void handleAction({ op: "assign", tracks: [t.id], characterId: c.id }, [t.id])}>
                        <Check /> Yes, {c.label}
                      </ContextMenuItem>
                      <ContextMenuItem onSelect={() => void handleAction({ op: "not_them", tracks: [t.id], characterId: c.id }, [t.id])}>
                        <X /> Not {c.label}
                      </ContextMenuItem>
                      <ContextMenuSeparator />
                      <ContextMenuItem onSelect={() => onOpen(t)}>
                        <Play /> Watch this moment
                      </ContextMenuItem>
                    </>
                  }
                >
                  <div className="group relative aspect-square overflow-hidden rounded-xl bg-inset picture-edge transition-transform duration-100 ease-out hover:scale-[1.03]">
                    <Tooltip label={media.get(t.mediaId)?.label ?? "This face"} hint="Double-click to watch" side="top" delay={600}>
                      <img
                        src={assetUrl(t.thumb)}
                        alt=""
                        loading="lazy"
                        className="size-full object-cover"
                        onDoubleClick={() => onOpen(t)}
                      />
                    </Tooltip>
                    <div className="absolute inset-x-1 bottom-1 flex justify-between gap-1 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                      <Tooltip label={`Yes, ${c.label}`} side="top">
                        <button
                          onClick={() => void handleAction({ op: "assign", tracks: [t.id], characterId: c.id }, [t.id])}
                          aria-label={`Yes, ${c.label}`}
                          className="flex h-7 flex-1 items-center justify-center rounded-lg bg-black/60 text-white backdrop-blur-md hover:bg-accent hover:text-on-accent active:scale-95 transition-transform"
                        >
                          <Check className="size-3.5" />
                        </button>
                      </Tooltip>
                      <Tooltip label={`Not ${c.label}`} side="top">
                        <button
                          onClick={() => void handleAction({ op: "not_them", tracks: [t.id], characterId: c.id }, [t.id])}
                          aria-label={`Not ${c.label}`}
                          className="flex h-7 flex-1 items-center justify-center rounded-lg bg-black/60 text-white backdrop-blur-md hover:bg-danger hover:text-white active:scale-95 transition-transform"
                        >
                          <X className="size-3.5" />
                        </button>
                      </Tooltip>
                    </div>
                  </div>
                </ContextArea>
              </motion.div>
            ))}
          </AnimatePresence>
        </div>
        {activeTracks.length > 12 && (
          <button onClick={() => setAll((v) => !v)} className="mt-4 text-sm font-medium text-fg-2 hover:text-fg-1">
            {all ? "Show fewer" : `Show ${plural(activeTracks.length - shown.length, "more face")}`}
          </button>
        )}
      </section>
    </ContextArea>
  )
}
