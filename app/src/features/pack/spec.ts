import type { PackChoices, PackClip, PackPreview, PackReason, PackSpec, ShotSize } from "@/bindings"

/**
 * The scenepack builder's filters and picks, as plain data. The backend
 * (as_project::pack) makes the pack; this only names its choices and shows
 * a click at once, before the backend's answer replaces it.
 */

export const DEFAULT_SPEC: PackSpec = {
  minSec: null,
  maxSec: null,
  sizes: [],
  solo: false,
  skipTiny: true,
  skipFaceless: false,
  media: [],
  order: "episodes",
  limit: null,
  handles: 0,
}

/** Shot sizes, closest first. */
export const SIZES: { value: ShotSize; label: string; short: string }[] = [
  { value: "extreme_close_up", label: "Extreme close-up", short: "ECU" },
  { value: "close_up", label: "Close-up", short: "CU" },
  { value: "medium_close_up", label: "Medium close-up", short: "MCU" },
  { value: "medium", label: "Medium shot", short: "MS" },
  { value: "wide", label: "Wide shot", short: "WS" },
]

/** Why a shot is left out, as its card says it. */
export const REASONS: Record<PackReason, string> = {
  missing: "File missing",
  episode: "Other episode",
  tiny: "Flash cut",
  short: "Too short",
  long: "Too long",
  size: "Other size",
  not_solo: "Not alone",
  faceless: "No face",
  limit: "Past the limit",
  removed: "Taken out",
}

/** Lengths to keep, in seconds. */
export const LENGTHS: { id: string; label: string; min: number | null; max: number | null }[] = [
  { id: "any", label: "Any length", min: null, max: null },
  { id: "1+", label: "1 s or longer", min: 1, max: null },
  { id: "2+", label: "2 s or longer", min: 2, max: null },
  { id: "1-6", label: "1 to 6 s", min: 1, max: 6 },
  { id: "2-10", label: "2 to 10 s", min: 2, max: 10 },
  { id: "-6", label: "Up to 6 s", min: null, max: 6 },
]

export const LIMITS: (number | null)[] = [null, 25, 50, 100, 200]
export const HANDLES: number[] = [0, 6, 12, 24]

/** The length choice `spec` makes (a range set elsewhere gets a label of its own). */
export function lengthOf(spec: PackSpec): { id: string | null; label: string } {
  const preset = LENGTHS.find((l) => l.min === spec.minSec && l.max === spec.maxSec)
  if (preset) return preset
  const s = (v: number) => `${Math.round(v * 10) / 10}`
  const label =
    spec.minSec != null && spec.maxSec != null
      ? `${s(spec.minSec)} to ${s(spec.maxSec)} s`
      : spec.minSec != null
        ? `${s(spec.minSec)} s or longer`
        : `Up to ${s(spec.maxSec ?? 0)} s`
  return { id: null, label }
}

export function isDefault(spec: PackSpec): boolean {
  return (
    spec.minSec === DEFAULT_SPEC.minSec &&
    spec.maxSec === DEFAULT_SPEC.maxSec &&
    spec.sizes.length === 0 &&
    spec.solo === DEFAULT_SPEC.solo &&
    spec.skipTiny === DEFAULT_SPEC.skipTiny &&
    (spec.skipFaceless ?? false) === DEFAULT_SPEC.skipFaceless &&
    spec.media.length === 0 &&
    spec.order === DEFAULT_SPEC.order &&
    spec.limit === DEFAULT_SPEC.limit &&
    spec.handles === DEFAULT_SPEC.handles
  )
}

/** A shot of the pack by its media and first frame — what picks by hand stick to. */
export const keyOf = (c: PackClip) => `${c.shot.mediaId}:${c.shot.start}`

/** A shot whose file is gone can't be taken. */
export const pickable = (c: PackClip) => c.reason !== "missing"

/**
 * Picks that take `clips` in (`included`) or out: by hand where the filters
 * say otherwise, back to the filters where they agree — so taking a shot
 * out and in again leaves no pick behind.
 */
export function picksFor(clips: PackClip[], included: boolean): { pick: PackClip[]; unpick: PackClip[] } {
  const pick: PackClip[] = []
  const unpick: PackClip[] = []
  for (const c of clips) {
    if (!pickable(c)) continue
    if (c.byRules === included) {
      if (c.manual != null) unpick.push(c)
    } else if (c.manual !== included) {
      pick.push(c)
    }
  }
  return { pick, unpick }
}

/** The shots from one card to another (both ends too), in the order shown. */
export function between(clips: PackClip[], from: string, to: string): PackClip[] {
  const a = clips.findIndex((c) => keyOf(c) === from)
  const b = clips.findIndex((c) => keyOf(c) === to)
  if (a < 0 || b < 0) return b < 0 ? [] : [clips[b]]
  return clips.slice(Math.min(a, b), Math.max(a, b) + 1)
}

/**
 * The preview with `clips` taken in or out at once, totals included. The
 * backend's answer follows within moments and replaces it — with what this
 * can't know: the reason a shot handed back to the filters is out, and the
 * next-best shot a limit lets in. Unchanged clips keep their identity, so
 * only the cards that changed draw again.
 */
export function withIncluded(
  p: PackPreview,
  clips: PackClip[],
  included: boolean,
  fpsOf: (mediaId: string) => number | null,
): PackPreview {
  return withPicks(p, new Map(clips.map((c) => [keyOf(c), included])), fpsOf)
}

/** The preview with shots (by [`keyOf`]) taken in or out; the same preview when nothing changes. */
export function withPicks(
  p: PackPreview,
  picks: ReadonlyMap<string, boolean>,
  fpsOf: (mediaId: string) => number | null,
): PackPreview {
  if (picks.size === 0) return p
  let changed = false
  const next = p.clips.map((c) => {
    const included = picks.get(keyOf(c))
    if (included === undefined || c.included === included || !pickable(c)) return c
    changed = true
    const manual = c.byRules === included ? null : included
    const reason: PackReason | null = included ? null : manual === false ? "removed" : null
    return { ...c, included, manual, reason }
  })
  return changed ? { ...p, clips: next, totals: totalsOf(next, fpsOf) } : p
}

/** Whether choices after a pick are a preview's: the same clips in the same order (else the pack changed otherwise). */
export const fits = (p: PackPreview, a: PackChoices) => p.shape === a.shape && p.clips.length === a.choices.length

/**
 * The preview as the backend has it after a pick: each clip's choice (in
 * the preview's order — see [`fits`]), the totals and the stamp. Clips
 * whose choice stays keep their identity, so only the cards that changed
 * draw again.
 */
export function withChoices(p: PackPreview, a: PackChoices): PackPreview {
  let changed = false
  const clips = p.clips.map((c, i) => {
    const { included, byRules, reason, manual } = a.choices[i]
    if (c.included === included && c.byRules === byRules && c.reason === reason && c.manual === manual) return c
    changed = true
    return { ...c, included, byRules, reason, manual }
  })
  return { ...p, clips: changed ? clips : p.clips, totals: a.totals, stamp: a.stamp }
}

export function totalsOf(clips: PackClip[], fpsOf: (mediaId: string) => number | null): PackPreview["totals"] {
  let n = 0
  let frames = 0
  let seconds = 0
  for (const c of clips) {
    if (!c.included) continue
    n += 1
    frames += c.end - c.start
    const fps = fpsOf(c.shot.mediaId)
    if (fps) seconds += (c.end - c.start) / fps
  }
  return { clips: n, frames, seconds }
}

/** What the grid shows. */
export type Show = "all" | "in" | "out"

export function shown(clips: PackClip[], show: Show): PackClip[] {
  if (show === "all") return clips
  return clips.filter((c) => c.included === (show === "in"))
}

/** Picks by hand in a preview: shots taken against the filters, either way. */
export const handPicked = (clips: PackClip[]) => clips.filter((c) => c.manual != null)
