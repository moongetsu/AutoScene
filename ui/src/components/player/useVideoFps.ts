import { useEffect, useState } from "react"
import { api, type VideoMeta } from "@/lib/api"

/**
 * Source frame rate for a project, probed once per id.
 *
 * fps is null while loading or when probing failed; callers hide frame
 * controls and fall back to HH:MM:SS.mmm rather than inventing frames.
 * Cached module-wide so Timeline, Gallery and Export share one request.
 */
const cache = new Map<string, VideoMeta>()
const inflight = new Map<string, Promise<VideoMeta | null>>()

export function useVideoFps(projectId: string | null | undefined): {
  fps: number | null
  meta: VideoMeta | null
} {
  const [meta, setMeta] = useState<VideoMeta | null>(() =>
    projectId ? (cache.get(projectId) ?? null) : null,
  )

  useEffect(() => {
    if (!projectId) {
      setMeta(null)
      return
    }
    const hit = cache.get(projectId)
    if (hit) {
      setMeta(hit)
      return
    }
    let cancelled = false
    let pending = inflight.get(projectId)
    if (!pending) {
      pending = api
        .videoMeta(projectId)
        .then((m) => {
          cache.set(projectId, m)
          return m
        })
        .catch(() => null)
        .finally(() => {
          inflight.delete(projectId)
        })
      inflight.set(projectId, pending)
    }
    void pending.then((m) => {
      if (!cancelled && m) setMeta(m)
    })
    return () => {
      cancelled = true
    }
  }, [projectId])

  const fps = meta?.fps && meta.fps > 0 ? meta.fps : null
  return { fps, meta }
}
