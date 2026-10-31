import { useCallback, useEffect, useRef, useState } from "react"
import { type Query, hashKey, keepPreviousData, useQuery } from "@tanstack/react-query"
import { toast } from "sonner"
import { create } from "zustand"
import type {
  CharacterId,
  ExportId,
  JobInfo,
  MediaItem,
  PackChoices,
  PackClip,
  PackNaming,
  PackPreview,
  PackSpec,
  PackState,
  ProjectId,
} from "@/bindings"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { isFinished, useActivity } from "@/state/activity"
import { fits, keyOf, pickable, picksFor, withChoices, withPicks } from "./spec"

/** How long the filters rest before the pack is made again, and before they are kept. */
const PREVIEW_MS = 150
const SAVE_MS = 400

function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    // The first value (the saved filters arriving) needs no wait.
    const t = window.setTimeout(() => setSettled(value), settled == null ? 0 : ms)
    return () => window.clearTimeout(t)
    // `settled` only decides the first wait.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [value, ms])
  return settled
}

const NO_PICKS: ReadonlyMap<string, boolean> = new Map()

const previews = (pid: ProjectId, cid: CharacterId) => [...qk.pack(pid, cid), "preview"] as const
const plans = (pid: ProjectId, cid: CharacterId) => [...qk.pack(pid, cid), "plan"] as const

/**
 * Put the choices a click left (`answer`, for the pack `spec` makes) on the
 * pack shown. It is fetched whole instead when it changed otherwise
 * meanwhile (an analysis), when it isn't in yet (new filters), or when a
 * fetch of it is on its way (an event asked for it). Packs of other filters
 * don't know the click: one on screen (the filters changed meanwhile) is
 * fetched again, and the ones not on screen, this one too by then, are
 * dropped. A fetch on its way may have read the pack before the click: it
 * starts over. Resolves once the grid has the pack as the backend has it.
 */
async function takeChoices(pid: ProjectId, cid: CharacterId, spec: PackSpec, answer: PackChoices): Promise<void> {
  const all = previews(pid, cid)
  const key = [...all, spec]
  const others = (q: Query) => q.queryHash !== hashKey(key)
  queryClient.removeQueries({ queryKey: all, type: "inactive" })
  await queryClient.cancelQueries({ queryKey: all, predicate: others })
  const again = queryClient.invalidateQueries({ queryKey: all, predicate: others })
  const shown = queryClient.getQueryData<PackPreview>(key)
  const fetching = queryClient.isFetching({ queryKey: key, exact: true }) > 0
  if (shown && !fetching && fits(shown, answer)) {
    queryClient.setQueryData(key, withChoices(shown, answer))
    await again
  } else {
    await queryClient.cancelQueries({ queryKey: key, exact: true })
    await Promise.all([again, queryClient.invalidateQueries({ queryKey: key, exact: true })])
  }
}

/**
 * Plans (names and sizes) once the clicks are in: the one on screen made
 * again — from the start, as a fetch of it on its way may be older than the
 * clicks — and the others dropped.
 */
async function refreshPlans(pid: ProjectId, cid: CharacterId): Promise<void> {
  queryClient.removeQueries({ queryKey: plans(pid, cid), type: "inactive" })
  await queryClient.cancelQueries({ queryKey: plans(pid, cid) })
  await queryClient.invalidateQueries({ queryKey: plans(pid, cid) })
}

/**
 * A character's scenepack while it is being built: the filters (as last
 * used, kept a moment after each change), the pack they make, its plan
 * (first names, folder and sizes) and the picks by hand. Mount it once per
 * character.
 */
export function usePack(pid: ProjectId, cid: CharacterId, media: Map<string, MediaItem>) {
  const saved = useQuery({ queryKey: [...qk.pack(pid, cid), "state"], queryFn: () => commands.packGet(pid, cid) })
  const [edited, setEdited] = useState<PackState | null>(null)
  const state = edited ?? saved.data ?? null

  // Kept for next time: a moment after the last change, and on leaving.
  const unsaved = useRef<PackState | null>(null)
  const save = useCallback(() => {
    const pack = unsaved.current
    unsaved.current = null
    if (!pack) return
    // The next visit starts from them (nothing announces a save).
    queryClient.setQueryData([...qk.pack(pid, cid), "state"], pack)
    commands.packSave(pid, cid, pack).catch((e) => console.warn("keeping the pack's filters:", e))
  }, [pid, cid])
  useEffect(() => {
    if (!edited) return
    unsaved.current = edited
    const t = window.setTimeout(save, SAVE_MS)
    return () => window.clearTimeout(t)
  }, [edited, save])
  useEffect(() => save, [save])

  const setSpec = useCallback(
    (spec: PackSpec) => setEdited((e) => ({ naming: (e ?? saved.data)!.naming, spec })),
    [saved.data],
  )
  const setNaming = useCallback(
    (naming: PackNaming) => setEdited((e) => ({ spec: (e ?? saved.data)!.spec, naming })),
    [saved.data],
  )

  const fpsOf = useCallback(
    (mediaId: string) => {
      const fps = media.get(mediaId)?.fps
      return fps ? fps.num / fps.den : null
    },
    [media],
  )

  // Clicks the backend hasn't made part of the pack yet: shown on whatever
  // pack arrives meanwhile, so a card never flips back and forth.
  const [picks, setPicks] = useState<ReadonlyMap<string, boolean>>(NO_PICKS)
  const select = useCallback((p: PackPreview) => withPicks(p, picks, fpsOf), [picks, fpsOf])

  const settled = useSettled(state, PREVIEW_MS)
  const preview = useQuery({
    queryKey: [...previews(pid, cid), settled?.spec],
    queryFn: () => commands.packPreview(pid, cid, settled!.spec),
    enabled: settled != null,
    placeholderData: keepPreviousData,
    select,
  })
  const plan = useQuery({
    queryKey: [...plans(pid, cid), settled?.spec, settled?.naming],
    queryFn: () => commands.packPlan(pid, cid, settled!.spec, settled!.naming),
    enabled: settled != null,
    placeholderData: keepPreviousData,
    // A template that doesn't work says so at once.
    retry: false,
  })

  // The grid shows the pack of the filters as they are: shots can be taken.
  const ready = state != null && settled === state && preview.data != null && !preview.isPlaceholderData

  // Clicks go one after another, in order. Each answer is the pack's
  // choices (picks change nothing else, so its shots aren't fetched
  // again); the last one also brings the plan up to date.
  const queue = useRef<Promise<unknown>>(Promise.resolve())
  const clicks = useRef(0)
  const [sending, setSending] = useState(0)

  /** Send a click's picks for the pack `spec` makes, showing `shown` at once. */
  const send = useCallback(
    async (spec: PackSpec, batches: [PackClip[], boolean | null][], shown: (readonly [string, boolean])[]) => {
      const click = ++clicks.current
      setPicks((m) => new Map([...m, ...shown]))
      setSending((n) => n + 1)
      const picks = batches
        .filter(([clips]) => clips.length > 0)
        .map(([clips, include]) => ({ shots: clips.map((c) => ({ mediaId: c.shot.mediaId, shot: c.shot.start })), include }))
      const run = queue.current.then(() => commands.packOverride(pid, cid, picks, spec))
      queue.current = run.catch(() => undefined)
      try {
        await takeChoices(pid, cid, spec, await run)
      } catch (e) {
        toast.error("Couldn't change the pack", { description: errorMessage(e) })
        // Whatever reached it: the pack as the backend has it now.
        await queryClient.invalidateQueries({ queryKey: previews(pid, cid) })
      }
      if (click === clicks.current) {
        // A click made meanwhile keeps its picks on screen until it is in.
        setPicks(NO_PICKS)
        // The pack's names and size too, before it can be exported.
        await refreshPlans(pid, cid)
      }
      setSending((n) => n - 1)
    },
    [pid, cid],
  )

  /** Take shots in or out of the pack: at once on screen, then for good. */
  const setIncluded = useCallback(
    async (clips: PackClip[], included: boolean) => {
      if (!ready || !settled) return
      const { pick, unpick } = picksFor(clips, included)
      if (pick.length + unpick.length === 0) return
      const shown = clips.filter(pickable).map((c) => [keyOf(c), included] as const)
      await send(settled.spec, [[pick, included], [unpick, null]], shown)
    },
    [ready, settled, send],
  )

  /** Hand shots back to the filters (what they say doesn't matter: also while they change). */
  const clearPicks = useCallback(
    async (clips: PackClip[]) => {
      if (!settled) return
      const picked = clips.filter((c) => c.manual != null)
      if (picked.length === 0) return
      const shown = picked.filter(pickable).map((c) => [keyOf(c), c.byRules] as const)
      await send(settled.spec, [[picked, null]], shown)
    },
    [settled, send],
  )

  // What is shown is what an export takes: only once the filters rest, the
  // pack made from them is in and every click has reached it (the
  // preview's stamp names exactly those clips), its names and size too.
  const current = ready && sending === 0 && !preview.isFetching

  return { state, saved, setSpec, setNaming, preview, plan, setIncluded, clearPicks, ready, current, fpsOf }
}

export type Pack = ReturnType<typeof usePack>

/**
 * The exports started from each pack while the app is open, by project and
 * character: a job says which export it renders, not whose pack that was.
 */
const started = create<Record<string, ExportId[]>>(() => ({}))
const packKey = (pid: ProjectId, cid: CharacterId) => `${pid}/${cid}`

/** Note an export the pack of `cid` started. */
export function packExportStarted(pid: ProjectId, cid: CharacterId, exportId: ExportId) {
  const key = packKey(pid, cid)
  started.setState((s) => ({ [key]: [...(s[key] ?? []), exportId] }))
}

/** The job of the pack's newest export while it waits or runs, if any. */
export function usePackExport(pid: ProjectId, cid: CharacterId): JobInfo | undefined {
  const ids = started((s) => s[packKey(pid, cid)])
  return useActivity((s) => {
    if (!ids) return undefined
    for (const id of s.order) {
      const j = s.jobs[id]
      if (j.exportId && ids.includes(j.exportId) && !isFinished(j)) return j
    }
    return undefined
  })
}
