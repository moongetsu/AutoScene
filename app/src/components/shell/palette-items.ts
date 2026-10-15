import type { Cast, MediaItem } from "@/bindings"
import { formatScreenTime, plural } from "@/features/cast/cast"
import { formatDuration } from "@/lib/format"
import { fileStem, mediaTitle } from "@/lib/media"

/** What the palette finds in the open project: its characters and episodes. */
export type ProjectEntry = {
  id: string
  label: string
  hint: string
  /** More words it is found by (episode numbers written other ways, the file). */
  keywords: string[]
  picture: string | null
}

/** The cast as listed (the most on screen first), hidden characters left out. */
export function characterEntries(cast: Cast | undefined): ProjectEntry[] {
  return (cast?.characters ?? [])
    .filter((c) => !c.hidden)
    .map((c) => ({
      id: c.id,
      label: c.label,
      hint: `${formatScreenTime(c.screenSec ?? 0)} · ${plural(c.shots, "shot")}`,
      keywords: c.name ? [] : ["unnamed"],
      picture: c.coverSmall,
    }))
}

/** Episodes in order, found by "S01E03", "s1e3", "1x03", "E03" or their file. */
export function episodeEntries(media: MediaItem[] | undefined): ProjectEntry[] {
  return (media ?? []).map((m) => ({
    id: m.id,
    label: mediaTitle(m),
    hint: m.status === "missing" ? "File missing" : m.durationSec ? formatDuration(m.durationSec) : "",
    keywords: episodeKeywords(m),
    picture: m.posterSmall ?? m.poster,
  }))
}

export function episodeKeywords(m: Pick<MediaItem, "season" | "episode" | "fileName">): string[] {
  const words = [fileStem(m.fileName)]
  const { season: s, episode: e } = m
  if (e != null) {
    const ee = String(e).padStart(2, "0")
    words.push(`E${ee}`, `Episode ${e}`)
    if (s != null) {
      const ss = String(s).padStart(2, "0")
      words.push(`S${ss}E${ee}`, `S${s}E${e}`, `${s}x${ee}`)
    }
  }
  return words
}
