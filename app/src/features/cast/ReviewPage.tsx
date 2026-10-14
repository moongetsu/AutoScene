import { useEffect, useMemo, useState } from "react"
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router"
import { ArrowLeft, Check, Play, Sparkles } from "lucide-react"
import type { Character, FaceTrackItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { Kbd } from "@/components/ui/kbd"
import { Page } from "@/components/ui/page"
import { Progress } from "@/components/ui/progress"
import { assetUrl } from "@/lib/media"
import { useCast, useCastTracks, useMediaList } from "@/lib/queries"
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

  const back = (
    <Link to="/p/$pid/cast" params={{ pid }} className="inline-flex items-center gap-1 text-xs text-fg-3 hover:text-fg-1">
      <ArrowLeft className="size-3.5" /> Cast
    </Link>
  )

  if (!queue.isPending && !current) {
    return (
      <Page>
        <div className="px-8 pt-6">{back}</div>
        <EmptyState
          icon={<Sparkles />}
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
          className="mt-10"
        />
      </Page>
    )
  }

  return (
    <Page>
      <div className="mx-auto flex max-w-[760px] flex-col px-8 pt-6 pb-10">
        {back}
        <div className="mt-3 flex items-baseline justify-between">
          <h1 className="text-2xl text-fg-1">Review faces</h1>
          <span className="timecode text-xs text-fg-3">
            {done} of {total}
            {skipped.size > 0 && <> · {skipped.size} skipped</>}
          </span>
        </div>
        <Progress className="mt-2" value={total > 0 ? done / total : null} />

        {current && (
          <div className="mt-8 grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] items-center gap-8">
            <figure className="flex flex-col items-center">
              <div className="aspect-square w-full max-w-[300px] overflow-hidden rounded-xl bg-inset shadow-2">
                <img key={current.id} src={assetUrl(current.thumb)} alt="The face to decide on" className="size-full object-cover" />
              </div>
              <figcaption className="mt-2 flex items-center gap-2 text-2xs text-fg-3">
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
                  className="inline-flex items-center gap-1 rounded-xs px-1 hover:bg-hover hover:text-fg-1"
                >
                  <Play className="size-3" /> Watch
                </button>
              </figcaption>
            </figure>
            <div>
              {suggestion ? (
                <Suggestion character={suggestion} track={current} />
              ) : (
                <p className="text-sm text-fg-2">Who is this?</p>
              )}
              <div className="mt-6 flex flex-col gap-2">
                {suggestion && (
                  <>
                    <Button
                      variant="primary"
                      disabled={busy}
                      onClick={() => void answer(current.id, () => castEdit(pid, { op: "confirm", tracks: [current.id] }))}
                    >
                      <Check /> Yes, {suggestion.label}
                      <Kbd keys={["Y"]} className="ml-auto opacity-70 [&_kbd]:border-on-accent/20 [&_kbd]:bg-on-accent/10 [&_kbd]:text-on-accent" />
                    </Button>
                    <Button
                      disabled={busy}
                      onClick={() =>
                        void answer(current.id, () =>
                          castEdit(pid, { op: "not_them", tracks: [current.id], characterId: suggestion.id }),
                        )
                      }
                    >
                      No, someone else <Kbd keys={["N"]} className="ml-auto" />
                    </Button>
                  </>
                )}
                <div className="grid grid-cols-2 gap-2">
                  <Button
                    variant="ghost"
                    disabled={busy}
                    onClick={() => void answer(current.id, () => castEdit(pid, { op: "not_a_face", tracks: [current.id] }))}
                  >
                    Not a face <Kbd keys={["X"]} className="ml-auto" />
                  </Button>
                  <Button variant="ghost" onClick={() => setSkipped((s) => new Set(s).add(current.id))}>
                    Skip <Kbd keys={["→"]} className="ml-auto" />
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}

        {current && picks.some(Boolean) && (
          <div className="mt-10">
            <h2 className="eyebrow mb-2">Or someone else</h2>
            <div className="flex flex-wrap gap-2">
              {picks.map((p, i) => p && (
                <button
                  key={p.id}
                  disabled={busy}
                  onClick={() =>
                    void answer(current.id, () => castEdit(pid, { op: "assign", tracks: [current.id], characterId: p.id }))
                  }
                  className={cn(
                    "flex items-center gap-2 rounded-md border border-hairline bg-panel py-1 pr-2.5 pl-1 text-left transition-colors hover:border-strong hover:bg-hover",
                    p.id === suggestion?.id && "border-accent-line",
                  )}
                >
                  <span className="size-7 shrink-0 overflow-hidden rounded-sm bg-inset">
                    {p.cover && <img src={assetUrl(p.cover)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className="max-w-[120px] truncate text-xs text-fg-1">{p.label}</span>
                  <Kbd keys={[String(i + 1)]} />
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </Page>
  )
}

function Suggestion({ character: c, track }: { character: Character; track: FaceTrackItem }) {
  const sure = (track.score ?? 0) >= 0.45 ? "Looks a lot like" : "Might be"
  return (
    <div>
      <p className="text-sm text-fg-2">{sure}</p>
      <div className="mt-2 flex items-center gap-3">
        <span className="size-14 shrink-0 overflow-hidden rounded-lg bg-inset shadow-1">
          {c.cover && <img src={assetUrl(c.cover)} alt="" className="size-full object-cover" />}
        </span>
        <span className={cn("text-xl", c.name ? "text-fg-1" : "text-fg-2")}>{c.label}</span>
      </div>
    </div>
  )
}
