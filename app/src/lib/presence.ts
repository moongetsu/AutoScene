import { useMemo } from "react"
import type { Character, Presence } from "@/bindings"

export type Lane = { character: Character; spans: Presence[]; frames: number }

/** Characters of one media, most on screen first. */
export function lanesOf(presence: Presence[], characters: Map<string, Character>): Lane[] {
  const by = new Map<string, Lane>()
  for (const p of presence) {
    const c = characters.get(p.characterId)
    if (!c || c.hidden) continue
    const lane = by.get(c.id) ?? { character: c, spans: [], frames: 0 }
    lane.spans.push(p)
    lane.frames += p.end - p.start
    by.set(c.id, lane)
  }
  return [...by.values()].sort((a, b) => b.frames - a.frames)
}

/** Characters whose faces are on screen anywhere in `start..end`. */
export function charactersIn(lanes: Lane[], start: number, end: number): Lane[] {
  return lanes.filter((l) => l.spans.some((s) => s.start < end && start < s.end))
}

/** The next time `lane` is on screen after `frame` (wrapping around to the first). */
export function nextAppearance(lane: Lane, frame: number): number | undefined {
  return (lane.spans.find((s) => s.start > frame) ?? lane.spans[0])?.start
}

/** Memoised map of a cast by id. */
export function useCharacterMap(characters: Character[] | undefined): Map<string, Character> {
  return useMemo(() => new Map((characters ?? []).map((c) => [c.id, c])), [characters])
}
