import { useEffect, useState } from "react"
import { commands } from "@/lib/ipc"

/**
 * How often the player asks where an indexing is: each time one small
 * keyframe is decoded beside it (about a tenth of a second of one core),
 * never the frames the indexing reads.
 */
export const STILL_EVERY_MS = 3000

/**
 * Where the indexing of `mediaId` is now, as a picture (a `data:` URL),
 * asked for every few seconds while `on` and the window is shown; `null`
 * until the first one comes. A failed or empty answer keeps the last one.
 */
export function useIndexingStill(pid: string, mediaId: string | undefined, on: boolean): string | null {
  // Whose picture it is: another episode's is never shown.
  const [still, setStill] = useState<{ of: string; url: string } | null>(null)
  useEffect(() => {
    if (!on || !mediaId) return
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const ask = async () => {
      if (!document.hidden) {
        const url = await commands.mediaIndexingStill(pid, mediaId).catch(() => null)
        if (stopped) return
        if (url) setStill({ of: mediaId, url })
      }
      if (!stopped) timer = setTimeout(() => void ask(), STILL_EVERY_MS)
    }
    void ask()
    return () => {
      stopped = true
      clearTimeout(timer)
    }
  }, [pid, mediaId, on])
  return still && still.of === mediaId ? still.url : null
}
