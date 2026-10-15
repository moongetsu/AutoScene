import { useCallback, useEffect, useRef, useState } from "react"
import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { toast } from "sonner"
import type { CharacterId, MediaItem, PackClip, PackNaming, PackPreview, PackSpec, PackState, ProjectId } from "@/bindings"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { picksFor, withIncluded } from "./spec"

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

/**
 * A character's scenepack while it is being built: the filters (as last
 * used, kept a moment after each change), the pack they make, its plan
 * (names and sizes) and the picks by hand. Mount it once per character.
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

  const settled = useSettled(state, PREVIEW_MS)
  const preview = useQuery({
    queryKey: [...qk.pack(pid, cid), "preview", settled?.spec],
    queryFn: () => commands.packPreview(pid, cid, settled!.spec),
    enabled: settled != null,
    placeholderData: keepPreviousData,
  })
  const plan = useQuery({
    queryKey: [...qk.pack(pid, cid), "plan", settled?.spec, settled?.naming],
    queryFn: () => commands.packPlan(pid, cid, settled!.spec, settled!.naming),
    enabled: settled != null,
    placeholderData: keepPreviousData,
    // A template that doesn't work says so at once.
    retry: false,
  })

  const fpsOf = useCallback(
    (mediaId: string) => {
      const fps = media.get(mediaId)?.fps
      return fps ? fps.num / fps.den : null
    },
    [media],
  )

  /** Take shots in or out of the pack: at once on screen, then for good. */
  const setIncluded = useCallback(
    async (clips: PackClip[], included: boolean) => {
      const { pick, unpick } = picksFor(clips, included)
      if (pick.length + unpick.length === 0) return
      const shown = [...qk.pack(pid, cid), "preview", settled?.spec]
      queryClient.setQueryData<PackPreview>(shown, (p) => p && withIncluded(p, clips, included, fpsOf))
      const keys = (cs: PackClip[]) => cs.map((c) => ({ mediaId: c.shot.mediaId, shot: c.shot.start }))
      try {
        if (pick.length > 0) await commands.packOverride(pid, cid, keys(pick), included)
        if (unpick.length > 0) await commands.packOverride(pid, cid, keys(unpick), null)
      } catch (e) {
        toast.error("Couldn't change the pack", { description: errorMessage(e) })
        void queryClient.invalidateQueries({ queryKey: qk.pack(pid, cid) })
      }
    },
    [pid, cid, settled, fpsOf],
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

  // What is shown is what an export takes: only once the filters rest and
  // the pack made from them is in (its stamp names exactly those clips).
  const current = state != null && settled === state && !preview.isPlaceholderData && !preview.isFetching

  return { state, saved, setSpec, setNaming, preview, plan, setIncluded, clearPicks, current, fpsOf }
}

export type Pack = ReturnType<typeof usePack>
