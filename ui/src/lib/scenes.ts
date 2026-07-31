import type { Person, Scene, Segment } from "@/lib/api"

/** Face-presence window for WITH (preview_* when set; else full plate). */
export function segmentFaceRange(segment: Segment): {
  startSec: number
  endSec: number
} {
  const start =
    segment.preview_start_sec != null ? segment.preview_start_sec : segment.start_sec
  const end =
    segment.preview_end_sec != null ? segment.preview_end_sec : segment.end_sec
  const a = Math.min(start, end)
  const b = Math.max(start, end)
  return {
    startSec: Math.max(0, a),
    endSec: Math.max(a + 0.05, b),
  }
}

/**
 * Co-cast for a clip: other people whose face windows actually overlap this
 * character's face presence by at least minOverlapSec. Never uses full-shot
 * plates alone — that would WITH reverse-shot halves of the same cut.
 */
export function coCastForSegment(
  segment: Segment,
  allSegments: Segment[],
  people: Person[],
  minOverlapSec = 0.2,
): Person[] {
  const ids = new Set<number>()
  const a = segmentFaceRange(segment)
  for (const s of allSegments) {
    if (s.person_id === segment.person_id) continue
    const b = segmentFaceRange(s)
    const overlap = Math.min(b.endSec, a.endSec) - Math.max(b.startSec, a.startSec)
    if (overlap >= minOverlapSec) ids.add(s.person_id)
  }
  if (!ids.size) return []
  const byId = new Map(people.map((p) => [p.person_id, p]))
  return [...ids]
    .sort((a, b) => a - b)
    .map((id) => byId.get(id))
    .filter((p): p is Person => Boolean(p))
}

type Interval = { start: number; end: number; personId: number; index: number }

function segmentIntervals(segments: Segment[]): Interval[] {
  return segments
    .map((s, index) => {
      const r = segmentFaceRange(s)
      return {
        start: r.startSec,
        end: r.endSec,
        personId: s.person_id,
        index,
      }
    })
    .filter((s) => s.end > s.start)
    .sort(
      (a, b) =>
        a.start - b.start || a.end - b.end || a.personId - b.personId,
    )
}

/** Client fallback if API omits scenes. Matches server connected-component rule:
 * different people's segments link only when overlap >= minPresenceSec. */
export function buildSharedScenesClient(
  segments: Segment[],
  gapSec = 0.35,
  minSec = 0.5,
  minPresenceSec = 0.2,
): Scene[] {
  void gapSec // legacy param; overlap graph replaces time-union glue

  const intervals = segmentIntervals(segments)
  if (!intervals.length) return []

  const parent = intervals.map((_, i) => i)

  const find = (i: number): number => {
    while (parent[i] !== i) {
      parent[i] = parent[parent[i]]
      i = parent[i]
    }
    return i
  }

  const union = (i: number, j: number) => {
    const ri = find(i)
    const rj = find(j)
    if (ri !== rj) parent[ri] = rj
  }

  for (let i = 0; i < intervals.length; i++) {
    const a = intervals[i]
    for (let j = i + 1; j < intervals.length; j++) {
      const b = intervals[j]
      if (a.personId === b.personId) continue
      const overlap = Math.min(a.end, b.end) - Math.max(a.start, b.start)
      if (overlap >= minPresenceSec) union(i, j)
    }
  }

  const buckets = new Map<number, number[]>()
  for (let i = 0; i < intervals.length; i++) {
    const root = find(i)
    const list = buckets.get(root) ?? []
    list.push(i)
    buckets.set(root, list)
  }

  const raw: Scene[] = []
  for (const indices of buckets.values()) {
    const starts = indices.map((i) => intervals[i].start)
    const ends = indices.map((i) => intervals[i].end)
    const start = Math.min(...starts)
    const end = Math.max(...ends)
    if (end - start < minSec) continue
    const personIds = [...new Set(indices.map((i) => intervals[i].personId))].sort(
      (a, b) => a - b,
    )
    raw.push({
      scene_id: 0,
      start_sec: start,
      end_sec: end,
      person_ids: personIds,
    })
  }

  raw.sort(
    (a, b) =>
      a.start_sec - b.start_sec ||
      a.end_sec - b.end_sec ||
      a.person_ids.join(",").localeCompare(b.person_ids.join(",")),
  )
  return raw.map((s, scene_id) => ({ ...s, scene_id }))
}

/** Hover / tile video range = full scene-cut plate (export bounds). */
export function segmentPreviewRange(segment: Segment) {
  const start = segment.start_sec
  const end = segment.end_sec
  return {
    startSec: Math.max(0, Math.min(start, end)),
    endSec: Math.max(Math.min(start, end) + 0.1, Math.max(start, end)),
  }
}

/** Still-frame anchor: real detection of the focus person when available. */
export function segmentStillTime(segment: Segment) {
  const { startSec, endSec } = segmentPreviewRange(segment)
  if (segment.face_time_sec != null) {
    return Math.min(Math.max(segment.face_time_sec, startSec), endSec)
  }
  const face = segmentFaceRange(segment)
  return Math.max(
    0,
    face.startSec +
      Math.min(0.85, Math.max(0.2, (face.endSec - face.startSec) * 0.4)),
  )
}
