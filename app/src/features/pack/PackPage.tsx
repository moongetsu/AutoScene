import { type ReactNode, useCallback, useLayoutEffect, useMemo, useRef, useState } from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import { toast } from "sonner"
import { ArrowLeft, User, Users } from "@/components/icons"
import type { CastEdit, Character, MediaItem, PackClip } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { ErrorState } from "@/components/ui/error-state"
import { Page } from "@/components/ui/page"
import { Segmented } from "@/components/ui/segmented"
import { TopBar } from "@/components/ui/topbar"
import { VirtualGrid } from "@/components/ui/virtual-grid"
import { castEdit, plural } from "@/features/cast/cast"
import { ProjectLoadError } from "@/features/project/ProjectLoadError"
import { firstFailure, useCast, useMediaList, useSettings } from "@/lib/queries"
import { PACK_SHORTCUTS, useShortcuts } from "@/lib/shortcuts"
import { type BodyVerdict, PackCard } from "./PackCard"
import { PackExportDialog } from "./PackExportDialog"
import { PackFilters } from "./PackFilters"
import { PackFooter } from "./PackFooter"
import { type Show, between, handPicked, keyOf, shown } from "./spec"
import { usePack } from "./use-pack"

const NO_CLIPS: PackClip[] = []
const NO_MEDIA: MediaItem[] = []

/** A character's scenepack: filters, every shot of them (the pack's bright), and Export. */
export function PackPage() {
  const { pid, cid } = useParams({ from: "/p/$pid/cast/$cid/pack" })
  // A pack's filters and picks are its character's: another one starts afresh.
  return <PackView key={cid} pid={pid} cid={cid} />
}

function PackView({ pid, cid }: { pid: string; cid: string }) {
  const castQuery = useCast(pid)
  const mediaQuery = useMediaList(pid)
  const character = castQuery.data?.characters.find((c) => c.id === cid)
  const media = mediaQuery.data ?? NO_MEDIA
  const mediaById = useMemo(() => new Map(media.map((m) => [m.id, m])), [media])
  const pack = usePack(pid, cid, mediaById)

  const back = { label: character?.label ?? "Character", to: `/p/${pid}/cast/${cid}` }
  const toolbar = <TopBar back={back} title={character ? `${character.label} · Scenepack` : "Scenepack"} />

  const failure = firstFailure(castQuery, mediaQuery, pack.saved)
  if (failure) {
    return (
      <Page topbar={toolbar}>
        <ProjectLoadError error={failure.error} onRetry={failure.retry} />
      </Page>
    )
  }
  if (castQuery.data && !character) {
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
  return <Builder pid={pid} character={character} media={media} mediaById={mediaById} pack={pack} toolbar={toolbar} />
}

function Builder({
  pid,
  character,
  media,
  mediaById,
  pack,
  toolbar,
}: {
  pid: string
  character: Character | undefined
  media: MediaItem[]
  mediaById: Map<string, MediaItem>
  pack: ReturnType<typeof usePack>
  toolbar: ReactNode
}) {
  const navigate = useNavigate()
  const { data: settings } = useSettings()
  const [show, setShow] = useState<Show>("all")
  const [exportOpen, setExportOpen] = useState(false)
  const clips = pack.preview.data?.clips ?? NO_CLIPS
  const totals = pack.preview.data?.totals ?? { clips: 0, frames: 0, seconds: 0 }
  const visible = useMemo(() => shown(clips, show), [clips, show])
  const picks = useMemo(() => handPicked(clips), [clips])
  const taken = totals.clips
  const canExport = taken > 0 && pack.current && character != null
  const bytes = pack.plan.data?.sizes.find((s) => s.presetId === settings?.exportPresetId)?.bytes ?? null

  useShortcuts(PACK_SHORTCUTS, { export: () => (canExport ? setExportOpen(true) : false) })

  // The cards call back with their index; these read the latest list (so
  // the cards, memoised, needn't draw again when it changes).
  const latest = useRef({ visible, setIncluded: pack.setIncluded, show })
  useLayoutEffect(() => {
    latest.current = { visible, setIncluded: pack.setIncluded, show }
  })
  // The last card clicked: Shift+click takes every shot from it.
  const anchor = useRef<string | null>(null)
  const toggle = useCallback((index: number, range: boolean, byKey?: boolean) => {
    const { visible, setIncluded, show } = latest.current
    const clip = visible[index]
    if (!clip) return
    const included = !clip.included
    const from = range ? anchor.current : null
    anchor.current = keyOf(clip)
    void setIncluded(from ? between(visible, from, keyOf(clip)) : [clip], included)
    // In the pack's or the left-out view the card leaves the grid: the
    // keyboard goes on with the one that takes its place.
    if (byKey && show !== "all") {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          const cards = document.querySelectorAll<HTMLElement>("[data-pack-index]")
          const next = [...cards].find((c) => c.dataset.packIndex === String(index)) ?? cards[cards.length - 1]
          next?.focus()
        }),
      )
    }
  }, [])
  const open = useCallback(
    (index: number) => {
      const clip = latest.current.visible[index]
      if (clip) void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: clip.shot.mediaId }, search: { f: clip.start } })
    },
    [navigate, pid],
  )
  // Bodies that aren't them (or aren't anyone) leave the pack, and stay out after a new analysis.
  const characterId = character?.id
  const label = character?.label
  const onBody = useCallback(
    (bodies: number[], verdict: BodyVerdict) => {
      if (!characterId) return
      const edit: CastEdit =
        verdict === "not_them" ? { op: "body_not_them", bodies, characterId } : { op: "not_a_person", bodies }
      void castEdit(pid, edit).then((r) => {
        if (r !== undefined) toast(verdict === "not_them" ? `Not ${label ?? "them"}: out of the pack` : "Not a person: out of the pack")
      })
    },
    [pid, characterId, label],
  )
  const faceless = useMemo(() => clips.some((c) => c.shot.faceless), [clips])

  return (
    <div className="relative h-full">
      <Page topbar={toolbar}>
        <Header character={character} shots={clips.length} />
        {pack.state ? (
          <PackFilters spec={pack.state.spec} media={media} faceless={faceless} onChange={pack.setSpec} />
        ) : (
          <div className="h-[57px] border-b border-hairline" />
        )}
        <section className="px-10 pt-5 pb-32">
          <div className="mb-4 flex flex-wrap items-center justify-between gap-4">
            <Segmented<Show>
              aria-label="Show"
              value={show}
              onChange={setShow}
              options={[
                { value: "all", label: `All ${clips.length}` },
                { value: "in", label: `In the pack ${taken}` },
                { value: "out", label: `Left out ${clips.length - taken}` },
              ]}
            />
            {clips.length > 0 && (
              <span className="text-sm text-fg-3">Click to take a shot in or out · Shift for every shot since · Enter to watch</span>
            )}
          </div>
          {pack.preview.isPending ? (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4">
              {Array.from({ length: 12 }, (_, i) => (
                <div key={i} className="skeleton aspect-video rounded-xl" />
              ))}
            </div>
          ) : pack.preview.isError && pack.preview.data === undefined ? (
            <ErrorState
              error={pack.preview.error}
              title="The pack couldn't be made"
              onRetry={() => void pack.preview.refetch()}
            />
          ) : clips.length === 0 ? (
            <p className="text-md text-fg-3">
              {character && character.waiting > 0
                ? "Their episode is being analysed again; the shots come back when it's done."
                : "No shots of them yet."}
            </p>
          ) : visible.length === 0 ? (
            <p className="text-md text-fg-3">{show === "in" ? "The pack takes none of the shots. Loosen a filter or click shots to take them." : "The pack takes every shot."}</p>
          ) : (
            <VirtualGrid
              items={visible}
              minItemWidth={240}
              itemHeight={(w) => (w * 9) / 16}
              getKey={keyOf}
              renderItem={(c, i) => (
                <PackCard
                  clip={c}
                  index={i}
                  media={mediaById.get(c.shot.mediaId)}
                  name={character?.label}
                  onToggle={toggle}
                  onOpen={open}
                  onBody={onBody}
                />
              )}
            />
          )}
        </section>
        {character && (
          <PackExportDialog pid={pid} character={character} pack={pack} open={exportOpen} onOpenChange={setExportOpen} />
        )}
      </Page>
      {clips.length > 0 && (
        <PackFooter
          totals={totals}
          bytes={bytes}
          picks={picks.length}
          canExport={canExport}
          onClearPicks={() => void pack.clearPicks(picks)}
          onExport={() => setExportOpen(true)}
        />
      )}
    </div>
  )
}

function Header({ character: c, shots }: { character?: Character; shots: number }) {
  return (
    <header className="flex items-center gap-5 px-10 pt-8 pb-6">
      <Artwork
        key={c?.id}
        path={c?.coverSmall}
        className="size-20 shrink-0 rounded-[20px] shadow-[0_20px_40px_-16px_rgb(0_0_0/0.8),inset_0_0_0_1px_rgb(255_255_255/0.12)]"
        imgClassName="object-[50%_30%]"
        fallback={
          <div className="flex size-full items-center justify-center bg-white/[0.06] text-white/60 [&_svg]:size-8">
            <User />
          </div>
        }
      />
      <div className="min-w-0">
        <p className="eyebrow">Scenepack</p>
        <h1 className="display truncate text-5xl leading-[1.05] text-fg-1">{c?.label ?? " "}</h1>
        {c && (
          <p className="mt-1 text-sm text-fg-3">
            {plural(shots, "shot")} in {plural(c.media, "episode")} · the pack takes the bright ones
          </p>
        )}
      </div>
    </header>
  )
}
