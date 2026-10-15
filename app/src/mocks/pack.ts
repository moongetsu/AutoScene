/**
 * Mock scenepacks: the backend's rules (as_project::pack) in short, over the
 * mock cast's shots, so the pack builder can be built and checked in a
 * plain browser. Same filters, reasons, order, limit and hand picks.
 */
import type {
  ExportPlan,
  MediaItem,
  PackClip,
  PackKey,
  PackNaming,
  PackPreview,
  PackReason,
  PackSpec,
  PackState,
  Segment,
  ShotSize,
} from "@/bindings"
import { characterLabel, characterShots, othersInShot } from "./cast"

const states = new Map<string, PackState>()
const picks = new Map<string, Map<string, boolean>>()

export function defaultPackState(): PackState {
  return {
    spec: { minSec: null, maxSec: null, sizes: [], solo: false, skipTiny: true, media: [], order: "episodes", limit: null, handles: 0 },
    naming: { template: "{series} {character} {episode} {tc}", folder: true },
  }
}

const keyOf = (pid: string, cid: string) => `${pid}:${cid}`

export function packGet(pid: string, cid: string): PackState {
  return structuredClone(states.get(keyOf(pid, cid)) ?? defaultPackState())
}

export function packSave(pid: string, cid: string, pack: PackState) {
  states.set(keyOf(pid, cid), structuredClone(pack))
}

export function packOverride(pid: string, cid: string, shots: PackKey[], include: boolean | null) {
  const m = picks.get(keyOf(pid, cid)) ?? new Map<string, boolean>()
  for (const s of shots) {
    if (include == null) m.delete(`${s.mediaId}:${s.shot}`)
    else m.set(`${s.mediaId}:${s.shot}`, include)
  }
  picks.set(keyOf(pid, cid), m)
}

function sizeOf(h: number): ShotSize {
  if (h >= 0.45) return "extreme_close_up"
  if (h >= 0.25) return "close_up"
  if (h >= 0.15) return "medium_close_up"
  if (h >= 0.08) return "medium"
  return "wide"
}

const lengthFit = (sec: number) => (sec < 1 ? sec : sec > 6 ? 6 / sec : 1)

export function packPreview(pid: string, cid: string, spec: PackSpec, media: MediaItem[], segments: Segment[]): PackPreview {
  const manual = picks.get(keyOf(pid, cid)) ?? new Map<string, boolean>()
  const order = new Map(media.map((m, i) => [m.id, i]))
  type Cand = { clip: PackClip; rule: PackReason | null; seconds: number }
  const cands: Cand[] = characterShots(pid, cid, media, segments).map((shot) => {
    const m = media.find((x) => x.id === shot.mediaId)!
    const fps = m.fps ? m.fps.num / m.fps.den : 24
    const seconds = (shot.end - shot.start) / fps
    const tiny = shot.end - shot.start < 3
    const others = othersInShot(pid, cid, shot.mediaId, shot.start, shot.end)
    const faceHeight = shot.faceHeight ?? 0
    const size = sizeOf(faceHeight)
    const quality = 0.6 + (shot.trackId % 4) * 0.1
    const score = 0.35 * quality + 0.3 * Math.min(faceHeight / 0.45, 1) + 0.2 + 0.1 * lengthFit(seconds) + (others === 0 ? 0.05 : 0)
    const rule: PackReason | null =
      m.status !== "ready"
        ? "missing"
        : spec.media.length > 0 && !spec.media.includes(m.id)
          ? "episode"
          : spec.skipTiny && tiny
            ? "tiny"
            : spec.minSec != null && seconds < spec.minSec
              ? "short"
              : spec.maxSec != null && seconds > spec.maxSec
                ? "long"
                : spec.sizes.length > 0 && !spec.sizes.includes(size)
                  ? "size"
                  : spec.solo && others > 0
                    ? "not_solo"
                    : null
    const frames = m.frames ?? shot.end
    const start = Math.max(0, shot.start - spec.handles)
    const end = Math.min(frames, shot.end + spec.handles)
    const pick = manual.get(`${shot.mediaId}:${shot.start}`)
    return {
      clip: { shot, start, end, size, quality, coverage: 1, others, score, included: false, byRules: rule == null, reason: rule, manual: pick ?? null },
      rule,
      seconds: (end - start) / fps,
    }
  })
  if (spec.limit != null) {
    const kept = cands.filter((c) => c.rule == null && c.clip.manual == null).sort((a, b) => (b.clip.score ?? 0) - (a.clip.score ?? 0))
    for (const c of kept.slice(spec.limit)) {
      c.rule = "limit"
      c.clip.byRules = false
    }
  }
  for (const c of cands) {
    const missing = c.rule === "missing"
    c.clip.included = !missing && (c.clip.manual ?? c.rule == null)
    c.clip.reason = c.clip.manual === false ? "removed" : c.clip.manual === true && !missing ? null : c.rule
  }
  const episode = (c: PackClip) => [order.get(c.shot.mediaId) ?? 1e9, c.shot.start] as const
  cands.sort((a, b) => {
    const [sa0, sb0] = [a.clip.score ?? 0, b.clip.score ?? 0]
    if (spec.order === "best" && sb0 !== sa0) return sb0 - sa0
    const [ea, sa] = episode(a.clip)
    const [eb, sb] = episode(b.clip)
    return ea - eb || sa - sb
  })
  const taken = cands.filter((c) => c.clip.included)
  return {
    clips: cands.map((c) => c.clip),
    totals: {
      clips: taken.length,
      frames: taken.reduce((s, c) => s + (c.clip.end - c.clip.start), 0),
      seconds: taken.reduce((s, c) => s + c.seconds, 0),
    },
    stamp: taken.map((c) => `${c.clip.shot.mediaId}:${c.clip.start}-${c.clip.end}`).join("|"),
  }
}

const NAME_TOKENS = ["series", "character", "episode", "index", "tc", "frame"]

/** What the backend says about a template that doesn't work (as_project::naming), or null. */
export function templateProblem(template: string): string | null {
  let rest = template
  for (let open = rest.indexOf("{"); open >= 0; open = rest.indexOf("{")) {
    const close = rest.indexOf("}", open)
    if (close < 0) return "A “{” isn't closed with “}”."
    const token = rest.slice(open + 1, close)
    if (!NAME_TOKENS.includes(token)) {
      return `“{${token}}” isn't a name part. Use {series}, {character}, {episode}, {index}, {tc} or {frame}.`
    }
    rest = rest.slice(close + 1)
  }
  return template.trim() === "" ? "The name would be empty." : null
}

/** The backend's template names, simply. */
export function packNames(
  pid: string,
  cid: string,
  title: string,
  preview: PackPreview,
  naming: PackNaming,
  media: MediaItem[],
  tcName: (f: number) => string,
): string[] {
  const taken = preview.clips.filter((c) => c.included)
  const width = Math.max(3, String(taken.length).length)
  return taken.map((c, i) => {
    const m = media.find((x) => x.id === c.shot.mediaId)
    const name = naming.template
      .replaceAll("{series}", title)
      .replaceAll("{character}", characterLabel(pid, cid))
      .replaceAll("{episode}", m?.label || m?.fileName.replace(/\.[^.]+$/, "") || "")
      .replaceAll("{index}", String(i + 1).padStart(width, "0"))
      .replaceAll("{tc}", tcName(c.start))
      .replaceAll("{frame}", String(c.start))
    return name.trim().replace(/[<>:"/\\|?*]+/g, " ").replace(/\s+/g, "_")
  })
}

export function packPlan(names: string[], preview: PackPreview, sizes: Record<string, number>): ExportPlan {
  return { names, sizes: Object.entries(sizes).map(([presetId, perFrame]) => ({ presetId, bytes: (preview.totals.frames ?? 0) * perFrame })) }
}
