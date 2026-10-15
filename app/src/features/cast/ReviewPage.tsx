import { useEffect, useMemo, useState } from "react"
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router"
import { Check, Checks, Play } from "@/components/icons"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { Kbd } from "@/components/ui/kbd"
import { Page } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { TopBar } from "@/components/ui/topbar"
import { assetUrl } from "@/lib/media"
import { portraitQuery, queryClient, useCast, useCastTracks, useMediaList, usePortrait } from "@/lib/queries"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { castEdit, plural } from "./cast"

/**
 * Faces the analysis wasn't sure about, one at a time: confirm the
 * suggestion, reject it, call it not a face, or give it to someone else.
 * Every answer is a pin or a forbid, so it survives any later analysis.
 */
export function ReviewPage() {
  const { pid } = useParams({ from: "/p/$pid/cast/review" })
  const { c } = useSearch({ from: "/p/$pid/cast/review" })
  const navigate = useNavigate()
  const { data: cast } = useCast(pid)
  const { data: media } = useMediaList(pid)
  const queue = useCastTracks(pid, c ? { kind: "unsure", id: c } : { kind: "all_unsure" })
  const [skipped, setSkipped] = useState<Set<number>>(new Set())
  // Answered faces leave at once, before the list is fetched again: a quick
  // second key must not land on the face just decided.
  const [answered, setAnswered] = useState<Set<number>>(new Set())
  const [done, setDone] = useState(0)
  const [busy, setBusy] = useState(false)
  // Quick picks 1–9: the biggest characters when the review started, in a
  // fixed order (numbers don't move as answers change the screen time).
  const [pickIds, setPickIds] = useState<string[] | null>(null)
  if (pickIds === null && cast) setPickIds(cast.characters.filter((x) => !x.hidden).slice(0, 9).map((x) => x.id))

  const items = (queue.data ?? []).filter((t) => !answered.has(t.id))
  const current = items.find((t) => !skipped.has(t.id))
  const byId = useMemo(() => new Map((cast?.characters ?? []).map((x) => [x.id, x])), [cast])
  const suggestion = current?.characterId ? byId.get(current.characterId) : undefined
  // A pick merged away meanwhile leaves its number empty.
  const picks = (pickIds ?? []).map((id) => byId.get(id))
  const m = current ? media?.find((x) => x.id === current.mediaId) : undefined
  const total = done + items.length
  // Shown big: the face's sharp portrait, the thumbnail until it's made.
  const portrait = usePortrait(pid, current)
  // The next faces' portraits are made while this one is decided.
  const upcoming = items.filter((t) => !skipped.has(t.id) && t.id !== current?.id).slice(0, 2)
  const upcomingIds = upcoming.map((t) => t.id).join(",")
  useEffect(() => {
    for (const t of upcoming) void queryClient.prefetchQuery(portraitQuery(pid, t))
    // Once per new pair of faces (`upcomingIds`), not per render.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [pid, upcomingIds])

  const answer = async (id: number, fn: () => Promise<unknown>) => {
    if (busy) return
    setBusy(true)
    setAnswered((s) => new Set(s).add(id))
    const r = await fn()
    setBusy(false)
    if (r !== undefined) {
      setDone((d) => d + 1)
    } else {
      setAnswered((s) => {
        const next = new Set(s)
        next.delete(id)
        return next
      })
    }
  }

  useEffect(() => {
    if (!current) return
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return
      if (e.target instanceof Element && e.target.closest("input, textarea, [role=dialog], [role=menu]")) return
      // A focused button answers Enter and Space itself.
      if ((e.key === "Enter" || e.key === " ") && e.target instanceof Element && e.target.closest("button, a")) return
      const k = e.key.toLowerCase()
      const id = current.id
      const target = /^[1-9]$/.test(k) ? picks[Number(k) - 1] : undefined
      if ((k === "y" || k === "j" || k === "enter") && suggestion) {
        void answer(id, () => castEdit(pid, { op: "confirm", tracks: [id] }))
      } else if (k === "n" && suggestion) {
        void answer(id, () => castEdit(pid, { op: "not_them", tracks: [id], characterId: suggestion.id }))
      } else if (k === "x") {
        void answer(id, () => castEdit(pid, { op: "not_a_face", tracks: [id] }))
      } else if (k === "s" || e.key === "ArrowRight") {
        setSkipped((s) => new Set(s).add(id))
      } else if (target) {
        void answer(id, () => castEdit(pid, { op: "assign", tracks: [id], characterId: target.id }))
      } else return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  })

  const toolbar = <TopBar back={{ label: "Cast", to: `/p/${pid}/cast` }} title="Review faces" />

  if (!queue.isPending && !current) {
    return (
      <Page topbar={toolbar}>
        <EmptyState
          icon={<Checks />}
          title={done > 0 ? "All reviewed" : "Nothing to review"}
          description={
            skipped.size > 0
              ? `${plural(skipped.size, "face")} skipped; they stay in the list for later.`
              : done > 0
                ? `${plural(done, "decision")} saved. They stick, whatever later analyses find.`
                : "Faces the analysis isn't sure about show up here."
          }
          actions={
            skipped.size > 0 ? (
              <Button onClick={() => setSkipped(new Set())}>Show skipped faces</Button>
            ) : (
              <Button asChild>
                <Link to="/p/$pid/cast" params={{ pid }}>
                  Back to the cast
                </Link>
              </Button>
            )
          }
          className="mt-24"
        />
      </Page>
    )
  }

  return (
    <Page topbar={toolbar}>
      {/* The face being decided lights the room. */}
      {current && (
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 h-screen overflow-hidden opacity-40">
          <img
            key={current.id}
            src={assetUrl(current.thumb)}
            alt=""
            className="size-full scale-125 animate-fade-in object-cover blur-[90px] saturate-150"
          />
          <div className="absolute inset-0 bg-gradient-to-b from-canvas/40 via-canvas/70 to-canvas" />
        </div>
      )}
      <div className="relative mx-auto flex max-w-[920px] flex-col px-10 pt-6 pb-12">
        <div className="flex items-center gap-4">
          <Progress className="flex-1" value={total > 0 ? done / total : null} />
          <span className="timecode shrink-0 text-sm text-fg-2">
            {done} of {total}
            {skipped.size > 0 && <span className="text-fg-3"> · {skipped.size} skipped</span>}
          </span>
        </div>

        {current && (
          <>
            <h2 className="display mt-[min(40px,4vh)] text-center text-5xl text-fg-1">
              {suggestion ? (
                <>
                  Is this <span className={cn(!suggestion.name && "italic")}>{suggestion.label}</span>?
                </>
              ) : (
                "Who is this?"
              )}
            </h2>
            {suggestion && (
              <p className="mt-2 text-center text-md text-fg-2">
                {(current.score ?? 0) >= 0.45 ? "Looks a lot like them" : "Might be them"} — the analysis wasn't sure enough to decide
                alone.
              </p>
            )}

            <div className="mt-[min(40px,4vh)] flex items-center justify-center gap-8">
              <figure className="flex flex-col items-center">
                <div className="size-[min(300px,32vh)] overflow-hidden rounded-[32px] bg-inset shadow-[0_40px_80px_-30px_rgb(0_0_0/0.9),inset_0_0_0_1px_rgb(255_255_255/0.12)]">
                  <img
                    key={current.id}
                    src={assetUrl(portrait.data ?? current.thumb)}
                    alt="The face to decide on"
                    className="size-full animate-fade-in object-cover object-[50%_30%]"
                  />
                </div>
                <figcaption className="mt-3 flex items-center gap-2 text-sm text-fg-3">
                  <span className="timecode">
                    {m?.label && `${m.label} · `}
                    {m?.fps ? timecode(current.start, m.fps, false) : current.start}
                  </span>
                  <button
                    onClick={() =>
                      void navigate({
                        to: "/p/$pid/timeline/$mid",
                        params: { pid, mid: current.mediaId },
                        search: { f: current.start },
                      })
                    }
                    className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 font-medium text-fg-2 transition-colors hover:bg-white/[0.08] hover:text-fg-1"
                  >
                    <Play weight="fill" className="size-3" /> Watch
                  </button>
                </figcaption>
              </figure>
              {suggestion && <Suggestion character={suggestion} />}
            </div>

            <div className="mt-[min(40px,4vh)] flex flex-wrap items-center justify-center gap-3">
              {suggestion && (
                <>
                  <Button
                    size="lg"
                    variant="primary"
                    disabled={busy}
                    onClick={() => void answer(current.id, () => castEdit(pid, { op: "confirm", tracks: [current.id] }))}
                  >
                    <Check weight="bold" /> Yes, {suggestion.label}
                    <Kbd keys={["Y"]} className="ml-1 [&_kbd]:border-black/10 [&_kbd]:bg-black/10 [&_kbd]:text-black/60" />
                  </Button>
                  <Button
                    size="lg"
                    variant="secondary"
                    disabled={busy}
                    onClick={() =>
                      void answer(current.id, () =>
                        castEdit(pid, { op: "not_them", tracks: [current.id], characterId: suggestion.id }),
                      )
                    }
                  >
                    No, someone else <Kbd keys={["N"]} className="ml-1" />
                  </Button>
                </>
              )}
              <Button
                size="lg"
                variant="ghost"
                disabled={busy}
                onClick={() => void answer(current.id, () => castEdit(pid, { op: "not_a_face", tracks: [current.id] }))}
              >
                Not a face <Kbd keys={["X"]} className="ml-1" />
              </Button>
              <Button size="lg" variant="ghost" onClick={() => setSkipped((s) => new Set(s).add(current.id))}>
                Skip <Kbd keys={["→"]} className="ml-1" />
              </Button>
            </div>
          </>
        )}

        {current && picks.some(Boolean) && (
          <div className="mt-14">
            <h3 className="mb-4 text-center text-md font-semibold text-fg-2">Or someone else</h3>
            <div className="flex flex-wrap justify-center gap-2.5">
              {picks.map(
                (p, i) =>
                  p && (
                    <button
                      key={p.id}
                      disabled={busy}
                      onClick={() =>
                        void answer(current.id, () => castEdit(pid, { op: "assign", tracks: [current.id], characterId: p.id }))
                      }
                      className={cn(
                        "flex items-center gap-2.5 rounded-full border border-white/10 bg-white/[0.05] py-1 pr-2 pl-1 text-left backdrop-blur-xl transition-colors hover:border-white/25 hover:bg-white/[0.1]",
                        p.id === suggestion?.id && "border-accent-line",
                      )}
                    >
                      <span className="size-8 shrink-0 overflow-hidden rounded-full bg-inset">
                        {p.coverSmall && <img src={assetUrl(p.coverSmall)} alt="" className="size-full object-cover" />}
                      </span>
                      <span className={cn("max-w-[140px] truncate text-sm font-medium", p.name ? "text-fg-1" : "text-fg-2")}>
                        {p.label}
                      </span>
                      <Kbd keys={[String(i + 1)]} />
                    </button>
                  ),
              )}
            </div>
          </div>
        )}
      </div>
    </Page>
  )
}

/** Who the analysis thinks it is, as big as the face being decided. */
function Suggestion({ character: c }: { character: Character }) {
  return (
    <>
      <span className="display text-4xl text-fg-3" aria-hidden>
        ?
      </span>
      <figure className="flex flex-col items-center">
        <div className="size-[min(300px,32vh)] overflow-hidden rounded-[32px] bg-inset shadow-[0_40px_80px_-30px_rgb(0_0_0/0.9),inset_0_0_0_1px_rgb(255_255_255/0.12)]">
          {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-full object-cover object-[50%_30%]" />}
        </div>
        <figcaption className={cn("display mt-3 text-2xl", c.name ? "text-fg-1" : "text-fg-2 italic")}>{c.label}</figcaption>
      </figure>
    </>
  )
}
