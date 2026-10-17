/**
 * Mock cast for the browser backend: characters with face tracks spread over
 * a project's episodes, edits that behave like the real ones (pins, forbids,
 * not-a-face, merges), and the derived views (presence, clips).
 */
import type {
  Cast,
  CastEdit,
  Character,
  CharacterShot,
  FaceTrackItem,
  MediaItem,
  Presence,
  Segment,
  TrackFilter,
} from "@/bindings"

type Track = FaceTrackItem & { forbid: Set<string>; notFace: boolean; portrait: string }
type Person = { id: string; number: number; name: string | null; hidden: boolean; cover: number | null }
type ProjectCast = { people: Person[]; tracks: Track[]; nextNumber: number }

const casts = new Map<string, ProjectCast>()
let trackSeq = 0
let personSeq = 0

/** A neutral face silhouette in the person's hue, varied per track. */
function face(hue: number, variant: number): string {
  const l = 26 + (variant % 5) * 3
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 160"><rect width="160" height="160" fill="hsl(${hue} 18% ${l}%)"/><circle cx="80" cy="66" r="30" fill="hsl(${hue} 30% ${l + 34}%)"/><path d="M28 160c4-34 26-52 52-52s48 18 52 52z" fill="hsl(${hue} 22% ${l + 20}%)"/></svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

/** The same face as a 3:4 head-and-shoulders portrait (what the real one frames). */
function portrait(hue: number, variant: number): string {
  const l = 26 + (variant % 5) * 3
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 150 200"><rect width="150" height="200" fill="hsl(${hue} 18% ${l}%)"/><circle cx="75" cy="80" r="34" fill="hsl(${hue} 30% ${l + 34}%)"/><path d="M14 200c4-46 28-70 61-70s57 24 61 70z" fill="hsl(${hue} 22% ${l + 20}%)"/></svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

/** A shot sprite: `cells` frames of someone walking across, in the person's hue. */
function sprite(hue: number, seed: number, cells = 8): string {
  const [w, h] = [240, 135]
  let body = ""
  for (let k = 0; k < cells; k++) {
    const x = k * w
    const l = 16 + ((seed + k) % 4) * 3
    const cx = x + 70 + k * 12
    body +=
      `<rect x="${x}" width="${w}" height="${h}" fill="hsl(${(hue + k * 5) % 360} 16% ${l}%)"/>` +
      `<circle cx="${cx}" cy="56" r="22" fill="hsl(${hue} 30% ${l + 36}%)"/>` +
      `<path d="M${cx - 48} ${h}c3-30 21-44 48-44s45 14 48 44z" fill="hsl(${hue} 22% ${l + 22}%)"/>`
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w * cells} ${h}">${body}</svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

const uuid = () => `0199c0de-0000-7000-8000-${(++personSeq).toString(16).padStart(12, "0")}`

/** Seed a cast for a live-action project (deterministic per call order). */
export function seedCast(projectId: string, media: MediaItem[], segments: Segment[]) {
  const shots = segments.filter((s) => s.kind === "shot")
  const people: Person[] = [
    { name: "Jonas", n: 26 },
    { name: "Martha", n: 18 },
    { name: null, n: 12 },
    { name: "Ulrich", n: 9 },
    { name: null, n: 6 },
    { name: null, n: 4 },
  ].map((p, i) => ({ id: uuid(), number: i + 1, name: p.name, hidden: false, cover: null, n: p.n }) as Person & { n: number })
  const tracks: Track[] = []
  const ready = media.filter((m) => m.status === "ready")
  const add = (who: Person | null, k: number, assign: Track["assign"], score: number, hue: number) => {
    const m = ready[k % ready.length]
    const s = shots[(k * 7 + (who?.number ?? 9)) % shots.length]
    const len = Math.max(6, Math.min(s.end - s.start, 24 + (k % 4) * 12))
    tracks.push({
      id: ++trackSeq,
      mediaId: m.id,
      start: s.start,
      end: s.start + len,
      quality: 0.35 + ((k * 37) % 60) / 100,
      faceHeight: 0.18 + (k % 5) * 0.04,
      thumb: face(hue, k),
      portrait: portrait(hue, k),
      characterId: who?.id ?? null,
      assign,
      score,
      forbid: new Set(),
      notFace: false,
    })
  }
  people.forEach((p, i) => {
    const hue = (i * 57 + 20) % 360
    for (let k = 0; k < (p as Person & { n: number }).n; k++) add(p, k + i * 3, k === 0 && p.name ? "pinned" : "auto", 1, hue)
    // Two faces that might be them.
    if (i < 3) for (let k = 0; k < 2; k++) add(p, 40 + k + i * 5, "unsure", 0.4, hue)
  })
  for (let k = 0; k < 14; k++) add(null, 60 + k, "none", 0.2, 200 + k * 9)
  if (ready.length === 0) return
  casts.set(projectId, { people, tracks, nextNumber: people.length + 1 })
}

function get(projectId: string): ProjectCast {
  let c = casts.get(projectId)
  if (!c) {
    c = { people: [], tracks: [], nextNumber: 1 }
    casts.set(projectId, c)
  }
  return c
}

const assigned = (t: Track) => !t.notFace && (t.assign === "auto" || t.assign === "pinned")

function character(c: ProjectCast, p: Person, fps = 24): Character {
  const mine = c.tracks.filter((t) => assigned(t) && t.characterId === p.id)
  const best = [...mine].sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0))[0]
  const cover = mine.find((t) => t.id === p.cover) ?? best
  return {
    id: p.id,
    name: p.name,
    label: p.name ?? `Character ${p.number}`,
    hidden: p.hidden,
    tracks: mine.length,
    shots: new Set(mine.map((t) => `${t.mediaId}:${t.start}`)).size,
    media: new Set(mine.map((t) => t.mediaId)).size,
    screenSec: mine.reduce((s, t) => s + (t.end - t.start), 0) / fps,
    cover: cover?.portrait ?? null,
    coverSmall: cover?.thumb ?? null,
    unsure: c.tracks.filter((t) => !t.notFace && t.assign === "unsure" && t.characterId === p.id).length,
    waiting: 0,
  }
}

/** Drop unnamed people nobody is in any more (like the real database). */
function tidy(c: ProjectCast) {
  c.people = c.people.filter((p) => p.name != null || c.tracks.some((t) => assigned(t) && t.characterId === p.id))
  const alive = new Set(c.people.map((p) => p.id))
  for (const t of c.tracks) {
    if (t.characterId && !alive.has(t.characterId)) {
      t.characterId = null
      t.assign = "none"
    }
  }
}

export function castGet(projectId: string, media: MediaItem[]): Cast {
  const c = get(projectId)
  const characters = c.people.map((p) => character(c, p)).sort((a, b) => (b.screenSec ?? 0) - (a.screenSec ?? 0))
  const live = c.tracks.filter((t) => !t.notFace)
  return {
    characters,
    unassigned: live.filter((t) => t.assign === "none").length,
    unsure: live.filter((t) => t.assign === "unsure").length,
    mediaDone: media.filter((m) => m.faces === "ready").length,
    mediaTotal: media.length,
  }
}

export function castTracks(projectId: string, filter: TrackFilter): FaceTrackItem[] {
  const c = get(projectId)
  const ok = (t: Track) => {
    if (t.notFace) return false
    switch (filter.kind) {
      case "character":
        return assigned(t) && t.characterId === filter.id
      case "unsure":
        return t.assign === "unsure" && t.characterId === filter.id
      case "all_unsure":
        return t.assign === "unsure"
      case "unassigned":
        return t.assign === "none"
    }
  }
  return c.tracks
    .filter(ok)
    .sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0))
    .map(({ forbid: _f, notFace: _n, portrait: _p, ...t }) => t)
}

/** A face's portrait (the mock has them all made). */
export function castPortrait(projectId: string, trackId: number): string | null {
  return get(projectId).tracks.find((t) => t.id === trackId)?.portrait ?? null
}

export function castEdit(projectId: string, edit: CastEdit): string | null {
  const c = get(projectId)
  const pick = (ids: number[]) => c.tracks.filter((t) => ids.includes(t.id))
  let created: string | null = null
  switch (edit.op) {
    case "assign":
      for (const t of pick(edit.tracks)) Object.assign(t, { characterId: edit.characterId, assign: "pinned", score: 1, notFace: false })
      break
    case "new_character": {
      const p: Person = { id: uuid(), number: c.nextNumber++, name: edit.name, hidden: false, cover: null }
      c.people.push(p)
      created = p.id
      for (const t of pick(edit.tracks)) Object.assign(t, { characterId: p.id, assign: "pinned", score: 1, notFace: false })
      break
    }
    case "not_them":
      for (const t of pick(edit.tracks)) {
        t.forbid.add(edit.characterId)
        if (t.characterId === edit.characterId) Object.assign(t, { characterId: null, assign: "none", score: 0 })
      }
      break
    case "not_a_face":
      for (const t of pick(edit.tracks)) Object.assign(t, { notFace: true, characterId: null, assign: "none" })
      break
    case "confirm":
      for (const t of pick(edit.tracks)) if (t.characterId) t.assign = "pinned"
      break
    case "rename": {
      const p = c.people.find((x) => x.id === edit.characterId)
      if (p) p.name = edit.name?.trim() || null
      break
    }
    case "set_hidden": {
      const p = c.people.find((x) => x.id === edit.characterId)
      if (p) p.hidden = edit.hidden
      break
    }
    case "merge": {
      const from = c.people.find((x) => x.id === edit.from)
      const into = c.people.find((x) => x.id === edit.into)
      if (!from || !into) throw { code: "not_found", message: "This character doesn't exist any more." }
      for (const t of c.tracks) {
        if (t.characterId !== from.id) continue
        t.characterId = into.id
        if (assigned(t)) t.assign = "pinned"
      }
      into.name ??= from.name
      c.people = c.people.filter((x) => x !== from)
      break
    }
    case "set_cover": {
      const p = c.people.find((x) => x.id === edit.characterId)
      if (p) p.cover = edit.track
      break
    }
    case "body_not_them":
    case "not_a_person":
      for (const b of edit.bodies) notThem.add(b)
      break
    case "body_assign":
      for (const b of edit.bodies) notThem.delete(b)
      break
  }
  tidy(c)
  return created
}

/** The setting to find characters by their bodies too, and bodies said not to be theirs. */
let findBodies = false
const notThem = new Set<number>()
const bodyIds = new Map<string, number>()

export function setMockBodies(on: boolean) {
  findBodies = on
}

/** With bodies found too, about one shot in six shows a character only from behind. */
function facelessIn(characterId: string, start: number): boolean {
  if (!findBodies) return false
  let h = start
  for (const ch of characterId) h = (h * 31 + ch.charCodeAt(0)) >>> 0
  return h % 6 === 5
}

function bodyId(mediaId: string, characterId: string, start: number): number {
  const key = `${mediaId}:${characterId}:${start}`
  let id = bodyIds.get(key)
  if (id === undefined) bodyIds.set(key, (id = 100000 + bodyIds.size))
  return id
}

export function mediaPresence(projectId: string, mediaId: string): Presence[] {
  const seen = new Set<number>()
  return get(projectId)
    .tracks.filter((t) => assigned(t) && t.mediaId === mediaId && t.characterId)
    .sort((a, b) => a.start - b.start)
    .flatMap((t): Presence[] => {
      const characterId = t.characterId!
      if (!facelessIn(characterId, t.start)) return [{ characterId, trackId: t.id, start: t.start, end: t.end }]
      // Their faces in the shot stand in for one body.
      const body = bodyId(mediaId, characterId, t.start)
      if (notThem.has(body) || seen.has(body)) return []
      seen.add(body)
      return [{ characterId, trackId: body, start: t.start, end: t.end, body: true }]
    })
}

export function characterShots(projectId: string, characterId: string, media: MediaItem[], segments: Segment[]): CharacterShot[] {
  const c = get(projectId)
  const hue = ((c.people.findIndex((p) => p.id === characterId) * 57 + 20) % 360 + 360) % 360
  const mine = c.tracks.filter((t) => assigned(t) && t.characterId === characterId)
  const out: CharacterShot[] = []
  let n = 0
  for (const m of media) {
    const here = mine.filter((t) => t.mediaId === m.id)
    for (const s of [...new Set(here.map((t) => t.start))].sort((a, b) => a - b)) {
      const seg = segments.find((x) => x.start === s && x.kind === "shot")
      if (!seg) continue
      const faces = here.filter((t) => t.start === s).sort((a, b) => (b.quality ?? 0) - (a.quality ?? 0))
      // Like media analysed before shots had sprites: a few shots of the first episode.
      const old = m === media[0] && n++ % 5 === 4
      const faceless = facelessIn(characterId, seg.start)
      const body = faceless ? bodyId(m.id, characterId, seg.start) : null
      if (body != null && notThem.has(body)) continue
      out.push({
        mediaId: m.id,
        start: seg.start,
        end: seg.end,
        sprite: old ? null : sprite(hue, s),
        cells: old ? 0 : 8,
        trackId: faceless ? 0 : faces[0].id,
        face: faces[0].thumb,
        faceHeight: faceless ? 0.3 : Math.max(...faces.map((t) => t.faceHeight ?? 0)),
        faceless,
        bodies: body != null ? [body] : [],
      })
    }
  }
  return out
}

/** How many other shown characters are on screen in a shot with `characterId`. */
export function othersInShot(projectId: string, characterId: string, mediaId: string, start: number, end: number): number {
  const c = get(projectId)
  const shown = new Set(c.people.filter((p) => !p.hidden && p.id !== characterId).map((p) => p.id))
  const here = c.tracks.filter(
    (t) => assigned(t) && t.mediaId === mediaId && t.start >= start && t.start < end && t.characterId && shown.has(t.characterId),
  )
  return new Set(here.map((t) => t.characterId)).size
}

/** A character's name, or "Character N". */
export function characterLabel(projectId: string, characterId: string): string {
  const p = get(projectId).people.find((x) => x.id === characterId)
  return p?.name ?? `Character ${p?.number ?? "?"}`
}

