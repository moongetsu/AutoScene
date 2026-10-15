import { useCallback, useEffect, useRef, useState } from "react"
import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { toast } from "sonner"
import type { CharacterId, MediaItem, PackClip, PackNaming, PackPreview, PackSpec, PackState, ProjectId } from "@/bindings"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { keyOf, pickable, picksFor, withPicks } from "./spec"

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
    queryKey: [...qk.pack(pid, cid), "preview", settled?.spec],
    queryFn: () => commands.packPreview(pid, cid, settled!.spec),
    enabled: settled != null,
    placeholderData: keepPreviousData,
    select,
  })
  const plan = useQuery({
    queryKey: [...qk.pack(pid, cid), "plan", settled?.spec, settled?.naming],
    queryFn: () => commands.packPlan(pid, cid, settled!.spec, settled!.naming),
    enabled: settled != null,
    placeholderData: keepPreviousData,
    // A template that doesn't work says so at once.
    retry: false,
  })

  // The grid shows the pack of the filters as they are: shots can be taken.
  const ready = state != null && settled === state && preview.data != null && !preview.isPlaceholderData

  // One click after another, in order; the last one to finish brings the
  // pack as the backend makes it now.
  const queue = useRef<Promise<unknown>>(Promise.resolve())
  const clicks = useRef(0)
  const [sending, setSending] = useState(0)

  /** Take shots in or out of the pack: at once on screen, then for good. */
  const setIncluded = useCallback(
    async (clips: PackClip[], included: boolean) => {
      if (!ready) return
      const { pick, unpick } = picksFor(clips, included)
      if (pick.length + unpick.length === 0) return
      const click = ++clicks.current
      const changed = clips.filter(pickable).map(keyOf)
      setPicks((m) => new Map([...m, ...changed.map((k) => [k, included] as const)]))
      setSending((n) => n + 1)
      const keys = (cs: PackClip[]) => cs.map((c) => ({ mediaId: c.shot.mediaId, shot: c.shot.start }))
      const run = queue.current.then(async () => {
        if (pick.length > 0) await commands.packOverride(pid, cid, keys(pick), included)
        if (unpick.length > 0) await commands.packOverride(pid, cid, keys(unpick), null)
      })
      queue.current = run.catch(() => undefined)
      try {
        await run
      } catch (e) {
        toast.error("Couldn't change the pack", { description: errorMessage(e) })
      }
      if (click === clicks.current) {
        await queryClient.refetchQueries({ queryKey: qk.pack(pid, cid) })
        // A click made meanwhile keeps its picks on screen until it is in.
        if (click === clicks.current) setPicks(NO_PICKS)
      }
      setSending((n) => n - 1)
    },
    [pid, cid, ready],
  )

  /** Hand every shot back to the filters. */
  const clearPicks = useCallback(
    async (clips: PackClip[]) => {
      const keys = clips.filter((c) => c.manual != null).map((c) => ({ mediaId: c.shot.mediaId, shot: c.shot.start }))
      if (keys.length === 0) return
      try {
        await commands.packOverride(pid, cid, keys, null)
      } catch (e) {
        toast.error("Couldn't change the pack", { description: errorMessage(e) })
      }
    },
    [pid, cid],
  )

  // What is shown is what an export takes: only once the filters rest, the
  // pack made from them is in and every click has reached it (the
  // preview's stamp names exactly those clips).
  const current = ready && sending === 0 && !preview.isFetching

  return { state, saved, setSpec, setNaming, preview, plan, setIncluded, clearPicks, ready, current, fpsOf }
}

export type Pack = ReturnType<typeof usePack>
