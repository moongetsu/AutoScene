/**
 * Browser mock backend. When the UI runs outside Tauri (plain `vite`, Vitest,
 * Playwright), IPC calls are answered here so every screen is reachable
 * without the Rust side. Keep handlers shaped exactly like the real commands —
 * `bindings.ts` types are the contract.
 *
 * Playback uses the barcoded fixture `cfr24_h264.mp4` (served by Vite from
 * `fixtures/out` when `cargo xtask fixtures` has been run), so frame stepping
 * can be checked against the burned-in frame numbers.
 */
import { emit } from "@tauri-apps/api/event"
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks"
import {
  castEdit,
  castGet,
  castPortrait,
  castTracks,
  characterLabel,
  characterShots,
  mediaPresence,
  setMockBodies,
  seedCast,
} from "./cast"
import {
  fileProblem,
  packChoices,
  packFileName,
  packGet,
  packNames,
  packOverride,
  packPlan,
  packPreview,
  packSave,
  templateProblem,
} from "./pack"
import type {
  AniListMedia,
  AniListMerge,
  AniListSuggestion,
  AniListSuggestions,
  AppInfo,
  PackExportRequest,
  PackNaming,
  PackPicks,
  PackSpec,
  PackState,
  AppSettings,
  ClipRequest,
  ContentKind,
  ExportItem,
  ExportPlan,
  ExportPreset,
  ExportRequest,
  ExportSummary,
  InstalledExtension,
  FrameMap,
  ImportCandidate,
  JobInfo,
  JobKind,
  MediaInfo,
  MediaItem,
  ModelInfo,
  NewProject,
  ProjectKind,
  ProjectSummary,
  Segment,
  TrackFilter,
  CastEdit,
  UpdateStatus,
} from "@/bindings"

type Handler = (args: Record<string, unknown>) => unknown

const FIXTURES = (import.meta.env.VITE_FIXTURES_URL as string | undefined) ?? ""
const VIDEO = `${FIXTURES}cfr24_h264.mp4`
const FRAMES = 586
const FPS = { num: 24, den: 1 }
const ROOT = String.raw`C:\Users\you\Documents\AutoScene Library`

/** What shot detection makes of the fixture (taken from a real run of the app). */
const SEGMENTS: Segment[] = (
  [
    [0, 48, "shot"],
    [48, 84, "shot"],
    [84, 145, "shot"],
    [145, 157, "dissolve"],
    [157, 205, "shot"],
    [205, 207, "fade_out"],
    [207, 213, "black"],
    [213, 220, "fade_in"],
    [220, 268, "shot"],
    [268, 269, "shot", true],
    [269, 271, "shot", true],
    [271, 274, "shot"],
    [274, 335, "shot"],
    [335, 346, "dissolve"],
    [346, 394, "shot"],
    [394, 406, "black"],
    [406, 479, "shot"],
    [479, 480, "fade_out"],
    [480, 483, "black"],
    [483, 490, "fade_in"],
    [490, 586, "shot"],
  ] as [number, number, Segment["kind"], boolean?][]
).map(([start, end, kind, tiny]) => ({ start, end, kind, tiny: tiny ?? false }))
const SHOT_COUNT = SEGMENTS.filter((s) => s.kind === "shot").length

const appInfo: AppInfo = {
  version: "1.0.3-BETA",
  platform: navigator.userAgent.includes("Mac") ? "macos" : "windows",
  arch: "x86_64",
  dataDir: String.raw`C:\Users\you\AppData\Local\AutoScene`,
  logsDir: String.raw`C:\Users\you\AppData\Local\AutoScene\logs`,
  libraryRoot: ROOT,
  extensionsDir: String.raw`C:\Users\you\AppData\Local\AutoScene\extensions`,
}

let mockClipboard: string | null = null
let settings: AppSettings = { libraryRoot: ROOT, exportPresetId: "prores-hq", audioLanguage: "eng", disabledExtensions: [] }

/** Canned AniList answers (no network in the mock). */
const ANILIST_SERIES: AniListMedia[] = [
  { id: 21519, title: "Frieren: Beyond Journey's End", format: "TV", episodes: 28, year: 2023 },
  { id: 172429, title: "Frieren: Beyond Journey's End Season 2", format: "TV", episodes: null, year: null },
]
const ANILIST_NAMES = ["Frieren", "Fern", "Stark", "Himmel"]

/** Per-project AniList decisions (the real backend keeps them in project meta). */
type MockAniList = {
  media: AniListMedia[]
  rejected: number[]
  auto: boolean
  rev: number
  /** The app picked the series from the title (not the person). */
  picked: boolean
  /** The look-up ran / runs, the names it left waiting, and those left out
   *  (`character:anilistId`). */
  looked: boolean
  looking: boolean
  suggested: { names: AniListSuggestion[]; merges: AniListMerge[] } | null
  refused: string[]
}
const anilistState = new Map<string, MockAniList>()
function anilist(projectId: string) {
  let s = anilistState.get(projectId)
  if (!s) {
    s = {
      media: [],
      rejected: [],
      auto: false,
      rev: 1,
      picked: false,
      looked: false,
      looking: false,
      suggested: null,
      refused: [],
    }
    anilistState.set(projectId, s)
  }
  return s
}
/** The canned names for a project's first unnamed characters (by AniList's
 *  order), like the matcher, which leaves named ones alone. */
function anilistMatch(p: MockProject, s: MockAniList): AniListSuggestions {
  const cast = castGet(String(p.id), p.media)
  const unnamed = cast.characters.filter((c) => !c.hidden && !c.name)
  const names: AniListSuggestion[] = unnamed.slice(0, ANILIST_NAMES.length).map((c, i) => ({
    characterId: c.id,
    anilistId: 1000 + i,
    name: ANILIST_NAMES[i],
    score: 0.9 - i * 0.1,
    image: null,
  }))
  return { rev: s.rev, names, merges: [], auto: s.auto }
}
function anilistAllowed() {
  if (!settings.anilist) throw { code: "invalid", message: "Turn on names from AniList in Settings first." }
}

/** Apply suggestions like `Library::anilist_apply`: merges first, then the
 * surviving characters' names. Returns how many were applied. */
function applyAnilist(
  p: MockProject,
  s: { rev: number },
  names: AniListSuggestion[],
  merges: AniListMerge[],
): number {
  const gone = new Set<string>()
  let applied = 0
  for (const m of merges) {
    castEdit(String(p.id), { op: "merge", from: m.from, into: m.into })
    gone.add(m.from)
    applied += 1
  }
  for (const n of names) {
    if (gone.has(n.characterId)) continue
    castEdit(String(p.id), { op: "rename", characterId: n.characterId, name: n.name })
    applied += 1
  }
  if (applied > 0) {
    s.rev += 1
    void emit("cast:changed", { projectId: p.id })
  }
  return applied
}

const presets: ExportPreset[] = [
  preset("prores-hq", "ProRes 422 HQ", "Edit-grade intra codec for After Effects, Premiere, Final Cut and Resolve. Large files.", "mov"),
  preset("dnxhr-hq", "DNxHR HQ", "Edit-grade intra codec, popular with Premiere and Resolve on Windows.", "mov"),
  preset("h264-edit", "H.264 · Editing", "Visually lossless H.264 with a short GOP for smooth scrubbing. Works everywhere.", "mp4"),
  preset("h264-mobile", "H.264 · Mobile (CapCut, Alight Motion)", "1080p H.264 with stereo sound that imports cleanly into phone editors.", "mp4"),
  preset("h264-intra", "H.264 · All-Intra (Twixtor)", "Every frame a keyframe — ideal for Twixtor and frame-by-frame work.", "mp4"),
  preset("hevc10", "HEVC 10-bit (keeps HDR)", "Smaller files that preserve 10-bit colour and HDR.", "mp4"),
  {
    ...preset("bit-perfect", "Bit-Perfect (lossless)", "Every frame exactly as in the episode: the same pixels, colour and HDR, and its own sound. Stored without loss as HEVC, which After Effects and other editors open (FFV1 for the rare pictures HEVC can't hold). Very large files.", "mov"),
    video: { codec: "lossless" },
    audio: { codec: "lossless" },
  },
]

function preset(id: string, name: string, description: string, container: "mov" | "mp4"): ExportPreset {
  return {
    id,
    name,
    description,
    container,
    video: { codec: "prores", profile: "hq" },
    audio: container === "mov" ? { codec: "pcm24" } : { codec: "aac", kbps: 320 },
    maxHeight: null,
    hdr: "auto",
    stereo: id === "h264-mobile",
    builtin: true,
  }
}

function info(codec: string, width: number, height: number, audio: [string | null, number][], hdr = false): MediaInfo {
  return {
    container: codec === "h264" ? "mov,mp4,m4a,3gp,3g2,mj2" : "matroska,webm",
    durationSec: FRAMES / 24,
    startSec: 0,
    sizeBytes: 0,
    bitRate: null,
    video: {
      index: 0,
      codec,
      profile: null,
      pixFmt: hdr ? "yuv420p10le" : "yuv420p",
      bitDepth: hdr ? 10 : 8,
      width,
      height,
      sar: { num: 1, den: 1 },
      rFrameRate: FPS,
      avgFrameRate: FPS,
      timeBase: { num: 1, den: 1000 },
      colorPrimaries: hdr ? "bt2020" : null,
      colorTransfer: hdr ? "smpte2084" : null,
      colorSpace: null,
      colorRange: null,
      fieldOrder: "progressive",
      rotation: 0,
      hdr: hdr ? "pq" : "none",
    },
    audio: audio.map(([language, channels], i) => ({
      index: i + 1,
      ordinal: i,
      codec: "aac",
      profile: null,
      channels,
      channelLayout: null,
      sampleRate: 48000,
      language,
      title: null,
      default: i === 0,
      commentary: false,
    })),
    subtitles: [],
  }
}

const now = Date.now()
let seq = 0
const id = () => `0199${(++seq).toString(16).padStart(4, "0")}-0000-7000-8000-${Math.random().toString(16).slice(2, 14).padEnd(12, "0")}`

function media(
  file: string,
  label: [number | null, number | null],
  extra: Partial<MediaItem> & { infoArgs?: Parameters<typeof info> },
  poster?: string,
): MediaItem {
  const [season, episode] = label
  const ready = extra.status === undefined || extra.status === "ready"
  const { infoArgs, ...rest } = extra
  return {
    id: id(),
    path: `D:\\Video\\${file}`,
    fileName: file,
    season,
    episode,
    label: episode == null ? "" : season == null ? `E${String(episode).padStart(2, "0")}` : `S${String(season).padStart(2, "0")}E${String(episode).padStart(2, "0")}`,
    position: 0,
    sizeBytes: 1.2e9 + Math.random() * 4e8,
    status: "ready",
    error: null,
    info: ready ? info(...(infoArgs ?? ["h264", 1920, 1080, [["jpn", 2]]])) : null,
    frames: ready ? FRAMES : null,
    durationSec: ready ? FRAMES / 24 : null,
    fps: ready ? FPS : null,
    vfr: false,
    interlaced: false,
    audioOrdinal: null,
    proxy: ready ? "not_needed" : "unknown",
    poster: ready && poster ? `${FIXTURES}posters/${poster}.jpg` : null,
    posterSmall: ready && poster ? `${FIXTURES}posters/${poster}.jpg` : null,
    addedAt: now,
    shots: ready ? "ready" : "pending",
    shotsError: null,
    shotCount: ready ? SHOT_COUNT : null,
    faces: ready ? "ready" : "pending",
    facesError: null,
    faceCount: ready ? 31 : null,
    ...rest,
  }
}

type MockProject = ProjectSummary & { media: MediaItem[]; exports: ExportSummary[] }

function project(title: string, kind: ProjectKind, content: ProjectSummary["content"], items: MediaItem[], age: number): MockProject {
  const p: MockProject = {
    id: id(),
    title,
    kind,
    content,
    dir: `${ROOT}\\${title}.autoscene`,
    createdAt: now - age,
    updatedAt: now - age,
    mediaCount: 0,
    readyCount: 0,
    durationSec: 0,
    poster: null,
    posterSmall: null,
    fastScan: false,
    media: items,
    exports: [],
  }
  refresh(p)
  return p
}

function refresh(p: MockProject) {
  p.media.forEach((m, i) => (m.position = i))
  p.mediaCount = p.media.length
  p.readyCount = p.media.filter((m) => m.status === "ready").length
  p.durationSec = p.media.reduce((s, m) => s + (m.status === "ready" ? (m.durationSec ?? 0) : 0), 0)
  const pictured = p.media.find((m) => m.poster)
  p.poster = pictured?.poster ?? null
  p.posterSmall = pictured?.posterSmall ?? null
  p.updatedAt = Date.now()
}

const projects: MockProject[] = [
  project(
    "Sousou no Frieren",
    "series",
    "anime",
    [
      media("[SubsPlease] Sousou no Frieren - 01 (1080p).mkv", [null, 1], { proxy: "ready", infoArgs: ["hevc", 1920, 1080, [["jpn", 2], ["eng", 6]]] }, "frieren-01"),
      media("[SubsPlease] Sousou no Frieren - 02 (1080p).mkv", [null, 2], { proxy: "ready", infoArgs: ["hevc", 1920, 1080, [["jpn", 2], ["eng", 6]]] }, "frieren-02"),
      media("[SubsPlease] Sousou no Frieren - 03 (1080p).mkv", [null, 3], { proxy: "ready", infoArgs: ["hevc", 1920, 1080, [["jpn", 2]]] }, "frieren-03"),
      media("[SubsPlease] Sousou no Frieren - 04 (1080p).mkv", [null, 4], { status: "indexing" }),
    ],
    1000,
  ),
  project(
    "Dark",
    "series",
    "live",
    [
      media("Dark.S01E01.Secrets.2160p.HDR.mkv", [1, 1], { proxy: "ready", infoArgs: ["hevc", 3840, 2160, [["ger", 6], ["eng", 6]], true] }, "dark-01"),
      media("Dark.S01E02.Lies.2160p.HDR.mkv", [1, 2], { proxy: "ready", infoArgs: ["hevc", 3840, 2160, [["ger", 6]], true] }, "dark-02"),
      media("Dark.S01E03.Past.and.Present.2160p.HDR.mkv", [1, 3], { status: "missing" }),
    ],
    86_400_000,
  ),
  project(
    "Blade Runner 2049",
    "movie",
    "live",
    [media("Blade Runner 2049 (2017).mp4", [null, null], { infoArgs: ["h264", 1920, 804, [["eng", 6]]] }, "br2049")],
    3 * 86_400_000,
  ),
]

for (const p of projects) seedCast(p.id, p.media, SEGMENTS)

function find(pid: unknown): MockProject {
  const p = projects.find((x) => x.id === pid)
  if (!p) throw { code: "not_found", message: "This project no longer exists." }
  return p
}

function findMedia(p: MockProject, mid: unknown): MediaItem {
  const m = p.media.find((x) => x.id === mid)
  if (!m) throw { code: "not_found", message: "This episode was removed." }
  return m
}

const summary = ({ media: _m, exports: _e, ...s }: MockProject): ProjectSummary => s

// -- jobs ---------------------------------------------------------------------

const jobs: JobInfo[] = []
/** Mirrors the backend's job revisions (every change gets a higher one). */
let rev = 0
/**
 * What a job's cancel leaves behind, by job, as the backend records it: a
 * cancelled indexing fails its media, a cancelled analysis is stopped.
 */
const onCancel = new Map<string, () => void>()

function startJob(
  kind: JobKind,
  title: string,
  p: MockProject,
  ms: number,
  opts: {
    mediaId?: string
    exportId?: string
    stage: string
    cancelled?: () => void
    /** Tells the job's stage, progress, time left and frames `elapsed` ms in, as its kind of work does. */
    tick?: (job: JobInfo, elapsed: number) => void
  },
  done: () => void,
) {
  const job: JobInfo = {
    id: id(),
    kind,
    title,
    subtitle: p.title,
    projectId: p.id,
    mediaId: opts.mediaId ?? null,
    exportId: opts.exportId ?? null,
    state: "running",
    progress: 0,
    stage: opts.stage,
    etaMs: ms,
    error: null,
    createdAt: Date.now(),
    finishedAt: null,
    rev: ++rev,
  }
  opts.tick?.(job, 0)
  jobs.unshift(job)
  if (opts.cancelled) onCancel.set(job.id, opts.cancelled)
  void emit("job:changed", { ...job })
  const t0 = Date.now()
  const timer = window.setInterval(() => {
    if (job.state !== "running") return window.clearInterval(timer)
    const elapsed = Date.now() - t0
    const f = Math.min(1, elapsed / ms)
    job.progress = f
    job.etaMs = Math.max(0, ms - elapsed)
    if (f < 1) opts.tick?.(job, elapsed)
    job.rev = ++rev
    const { progress, stage, etaMs, frames } = job
    void emit("job:progress", { id: job.id, rev: job.rev, progress, stage, etaMs, frames: frames ?? null })
    if (f >= 1) {
      window.clearInterval(timer)
      job.state = "done"
      job.finishedAt = Date.now()
      job.etaMs = null
      job.frames = null
      job.rev = ++rev
      onCancel.delete(job.id)
      done()
      void emit("job:changed", { ...job })
    }
  }, 250)
}

/**
 * Cancel the running jobs `of` matches. `record` false: without what a
 * cancel leaves behind (a restart or a removal follows, as in the backend).
 */
function cancelJobs(of: (j: JobInfo) => boolean, record = true) {
  for (const j of jobs) {
    if (j.state !== "running" || !of(j)) continue
    j.state = "cancelled"
    j.rev = ++rev
    j.finishedAt = Date.now()
    const after = onCancel.get(j.id)
    onCancel.delete(j.id)
    void emit("job:changed", { ...j })
    if (record) after?.()
  }
}

function ingest(p: MockProject, m: MediaItem) {
  m.status = "indexing"
  const cancelled = () => {
    Object.assign(m, { status: "failed", error: "Indexing was cancelled." })
    refresh(p)
    void emit("media:changed", { projectId: p.id, mediaId: m.id })
    void emit("library:changed", { projectId: p.id })
  }
  const title = `Indexing ${m.label ? `${m.label} · ` : ""}${m.fileName}`
  startJob("ingest", title, p, INDEX_MS, { mediaId: m.id, stage: "Reading file", cancelled, tick: indexingTick }, () => {
    Object.assign(m, media(m.fileName, [m.season, m.episode], {}), { id: m.id, path: m.path, poster: null, posterSmall: null })
    m.shots = "pending"
    m.shotCount = null
    refresh(p)
    void emit("media:changed", { projectId: p.id, mediaId: m.id })
    void emit("library:changed", { projectId: p.id })
    analyze(p, m)
  })
}

/** How long the mock indexes an episode: long enough to see its pace settle. */
const INDEX_MS = 24_000
/** A 23:40 episode at 23.976 fps: the frames its file states, and the ones read. */
const STATED_FRAMES = 34_046
const READ_FRAMES = 34_044
/** Where the mock's indexing is, as a picture: stills of the episode being indexed (Frieren 04). */
const INDEXING_STILLS = ["frieren-01", "frieren-02", "frieren-03"]

/**
 * An indexing as the backend tells it: the file read first, then its frames
 * counted (their pace told once measured for 2 s, wavering a little as a
 * real one does), then the poster made.
 */
function indexingTick(job: JobInfo, elapsed: number) {
  const [from, to] = [400, INDEX_MS * 0.95]
  job.frames = null
  if (elapsed < from || elapsed >= to) {
    job.stage = elapsed < from ? "Reading file" : "Creating poster"
    job.etaMs = null
    return
  }
  const read = (elapsed - from) / (to - from)
  const pace = READ_FRAMES / ((to - from) / 1000)
  const perSec = elapsed - from >= 2000 ? pace * (1 + 0.02 * Math.sin(elapsed / 900)) : null
  job.stage = "Indexing frames"
  job.progress = read * 0.95
  job.etaMs = perSec == null ? null : INDEX_MS - elapsed
  job.frames = { done: Math.round(READ_FRAMES * read), total: STATED_FRAMES, rate: 24000 / 1001, perSec }
}

function analyze(p: MockProject, m: MediaItem) {
  const facesOnly = m.shots === "ready"
  if (facesOnly) m.faces = "running"
  else m.shots = "running"
  const stage = facesOnly ? "Finding characters" : "Finding shots"
  const cancelled = () => {
    if (m.shots === "running") m.shots = "stopped"
    if (m.faces === "running") m.faces = "stopped"
    void emit("media:changed", { projectId: p.id, mediaId: m.id })
  }
  startJob("analyze", `Analysing ${m.label ? `${m.label} · ` : ""}${m.fileName}`, p, 4000, { mediaId: m.id, stage, cancelled }, () => {
    m.shots = "ready"
    m.shotCount = SHOT_COUNT
    m.faces = "ready"
    m.faceCount = 31
    void emit("media:changed", { projectId: p.id, mediaId: m.id })
    void emit("analysis:changed", { projectId: p.id, mediaId: m.id })
    void emit("cast:changed", { projectId: p.id })
  })
  void emit("media:changed", { projectId: p.id, mediaId: m.id })
}

// -- exports ------------------------------------------------------------------

function tcName(f: number) {
  const s = Math.floor(f / 24)
  const pad = (v: number) => String(v).padStart(2, "0")
  return `${pad(Math.floor(s / 3600))}-${pad(Math.floor(s / 60) % 60)}-${pad(s % 60)}-${pad(f % 24)}`
}

/** The backend's default clip name: `Title_S01E03_00-00-02-00`. */
function clipName(p: MockProject, c: ClipRequest): string {
  const m = findMedia(p, c.mediaId)
  return c.name ?? `${p.title}_${m.label || m.fileName.replace(/\.[^.]+$/, "")}_${tcName(c.start)}`.replace(/\s+/g, "_")
}

/** Rough bytes per frame for a preset (ProRes HQ 1080p ≈ 920 KB). */
const BYTES_PER_FRAME: Record<string, number> = {
  "prores-hq": 9.2e5,
  "dnxhr-hq": 9.1e5,
  "h264-edit": 1.0e5,
  "h264-mobile": 6.0e4,
  "h264-intra": 2.6e5,
  hevc10: 6.0e4,
  "bit-perfect": 9.3e5,
}

function exportPlan(p: MockProject, clips: ClipRequest[]): ExportPlan {
  const frames = clips.reduce((sum, c) => sum + (c.end - c.start), 0)
  return {
    names: clips.map((c) => clipName(p, c)),
    sizes: presets.map((pr) => ({ presetId: pr.id, bytes: frames * (BYTES_PER_FRAME[pr.id] ?? 2e5), apart: null, ext: null, note: null })),
    folder: null,
    file: null,
  }
}

function startExport(p: MockProject, req: ExportRequest): ExportSummary {
  const pr = presets.find((x) => x.id === req.presetId) ?? presets[0]
  const outDir = req.outDir ?? `${ROOT}\\Exports\\${p.title}`
  const items: ExportItem[] = req.clips.map((c, i) => {
    const name = clipName(p, c)
    return { id: i + 1, mediaId: c.mediaId, start: c.start, end: c.end, name, status: "queued", path: null, verified: false, encoder: null, bytes: null, error: null }
  })
  const merged = req.merge ? { name: req.merge, path: null, verified: false, bytes: null, error: null } : null
  const ex: ExportSummary = { id: id(), projectId: p.id, createdAt: Date.now(), preset: pr, outDir, status: "running", items, label: req.label, merged }
  p.exports.unshift(ex)
  items.forEach((it) => (it.status = "running"))
  const title = `${items.length === 1 ? "Exporting 1 clip" : `Exporting ${items.length} clips`}${merged ? " into one file" : ""}`
  startJob("export", title, p, 3500, { exportId: ex.id, stage: "Rendering" }, () => {
    for (const it of items) {
      it.status = "done"
      it.verified = true
      it.path = merged ? `${outDir}\\${merged.name}.${pr.container}` : `${outDir}\\${it.name}.${pr.container}`
      it.encoder = pr.container === "mov" ? "prores_ks" : "h264_nvenc"
      it.bytes = merged ? null : (it.end - it.start) * 1.4e6
    }
    if (merged) {
      ex.merged = { ...merged, path: `${outDir}\\${merged.name}.${pr.container}`, verified: true, bytes: items.reduce((s, it) => s + (it.end - it.start) * 1.4e6, 0) }
    }
    ex.status = "done"
    void emit("export:changed", { projectId: p.id, exportId: ex.id })
  })
  return ex
}

// Seed: finished exports so the Exports screen has history (one file, then a file per clip).
projects[0].exports.push(seedMergedExport(projects[0]), seedExport(projects[0]))

function seedMergedExport(p: MockProject): ExportSummary {
  const m = p.media[0]
  const path = `${ROOT}\\Exports\\${p.title}\\Sousou_no_Frieren_Himmel_scenepack.mov`
  const item = (i: number, start: number, end: number): ExportItem => ({
    id: i,
    mediaId: m.id,
    start,
    end,
    name: `Sousou_no_Frieren_Himmel_E01_${tcName(start)}`,
    status: "done",
    path,
    verified: true,
    encoder: "prores_ks",
    bytes: null,
    error: null,
  })
  return {
    id: id(),
    projectId: p.id,
    createdAt: now - 1_800_000,
    label: "Himmel · scenepack",
    preset: presets[0],
    outDir: `${ROOT}\\Exports\\${p.title}`,
    status: "done",
    items: [item(1, 84, 144), item(2, 220, 268), item(3, 346, 394)],
    merged: { name: "Sousou_no_Frieren_Himmel_scenepack", path, verified: true, bytes: 1.9e8, error: null },
  }
}

function seedExport(p: MockProject): ExportSummary {
  const m = p.media[0]
  return {
    id: id(),
    projectId: p.id,
    createdAt: now - 3_600_000,
    label: "Frieren · scenepack",
    preset: presets[0],
    outDir: `${ROOT}\\Exports\\${p.title}`,
    status: "done",
    items: [
      { id: 1, mediaId: m.id, start: 48, end: 84, name: `Sousou_no_Frieren_E01_${tcName(48)}`, status: "done", path: `${ROOT}\\Exports\\x.mov`, verified: true, encoder: "prores_ks", bytes: 5.2e7, error: null },
      { id: 2, mediaId: m.id, start: 156, end: 204, name: `Sousou_no_Frieren_E01_${tcName(156)}`, status: "done", path: `${ROOT}\\Exports\\y.mov`, verified: true, encoder: "prores_ks", bytes: 6.9e7, error: null },
    ],
    merged: null,
  }
}

// -- import -------------------------------------------------------------------

function scan(paths: string[], kind: ProjectKind): ImportCandidate[] {
  const files = paths.flatMap((p) =>
    /\.(mkv|mp4|mov|ts|m2ts|avi|webm)$/i.test(p)
      ? [p]
      : [1, 2, 3, 4, 5, 6].map((n) => `${p}\\${p.split(/[\\/]/).pop()} - ${String(n).padStart(2, "0")} [1080p].mkv`),
  )
  return files.map((path) => {
    const fileName = path.split(/[\\/]/).pop()!
    const m = kind === "series" ? (/S(\d+)E(\d+)/i.exec(fileName) ?? /-\s*(\d{1,4})\b/.exec(fileName)) : null
    const season = m && m.length === 3 ? Number(m[1]) : null
    const episode = m ? Number(m[m.length - 1]) : null
    const label = episode == null ? "" : season == null ? `E${String(episode).padStart(2, "0")}` : `S${String(season).padStart(2, "0")}E${String(episode).padStart(2, "0")}`
    return { path, fileName, season, episode, label, sizeBytes: 1.1e9 + Math.random() * 3e8, duplicate: false, refused: null }
  })
}

/** `models/manifest.toml`, as the `models` command lists it. */
const MODELS: ModelInfo[] = [
  { id: "transnetv2", purpose: "Shot boundaries (hard cuts and gradual transitions)", source: "TransNetV2 (Souček & Lokoč), ONNX export of the original weights", license: "MIT", nonCommercial: false, sizeBytes: 31250929, installed: true },
  { id: "scrfd-10g", purpose: "Face detection with five landmarks (live action)", source: "InsightFace buffalo_l det_10g (SCRFD-10G, Guo et al. 2021), byte-identical mirror by Immich", license: "InsightFace non-commercial research", nonCommercial: true, sizeBytes: 16923827, installed: true },
  { id: "arcface-glintr100", purpose: "Face identity embeddings (live action)", source: "InsightFace antelopev2 glintr100 (ArcFace ResNet-100 on Glint360K), byte-identical mirror by Immich", license: "InsightFace non-commercial research", nonCommercial: true, sizeBytes: 260665334, installed: true },
  { id: "yunet-2023mar", purpose: "Face detection with five landmarks (Fast Scan)", source: "OpenCV Zoo YuNet 2023mar (Wu et al. 2023), Hugging Face opencv/face_detection_yunet", license: "MIT", nonCommercial: false, sizeBytes: 232589, installed: false },
  { id: "sface-2021dec", purpose: "Face identity embeddings (Fast Scan)", source: "OpenCV Zoo SFace 2021dec (Zhong et al. 2021), Hugging Face opencv/face_recognition_sface", license: "Apache-2.0", nonCommercial: false, sizeBytes: 38696353, installed: false },
  { id: "person-ppyoloe", purpose: "People (whole bodies), to follow characters while their face doesn't show", source: "InsightFace cheetah_l pp_det_med: PaddleDetection PP-YOLOE pedestrian detector", license: "InsightFace non-commercial (cheetah_l-public-v1)", nonCommercial: true, sizeBytes: 29836627, installed: false },
  { id: "person-reid-0277", purpose: "How a person looks (clothes, build), to find them again without a face", source: "InsightFace cheetah_l reid_0265: OpenVINO person-reidentification-retail-0277 (OmniScaleNet)", license: "InsightFace non-commercial (cheetah_l-public-v1)", nonCommercial: true, sizeBytes: 8440242, installed: false },
]

// -- handlers -----------------------------------------------------------------

const handlers: Record<string, Handler> = {
  app_info: () => appInfo,
  reveal_path: () => null,
  // The app's own clipboard while in the browser (reading the browser's asks
  // for permission); copies also reach the system clipboard when it allows.
  clipboard_write: ({ text }) => {
    mockClipboard = String(text)
    void navigator.clipboard?.writeText(mockClipboard).catch(() => {})
    return null
  },
  clipboard_read: () => mockClipboard,
  open_logs_dir: () => null,
  menu_set_keys: () => null,
  media_tools: () => ({ ready: true, version: "8.0.1-full_build-www.gyan.dev", path: String.raw`C:\ffmpeg\bin\ffmpeg.exe`, error: null }),
  models: () => MODELS,
  settings_get: () => settings,
  settings_update: ({ settings: s }) => {
    settings = s as AppSettings
    setMockBodies(settings.findBodies ?? false)
    return settings
  },
  projects_list: () => [...projects].sort((a, b) => b.updatedAt - a.updatedAt).map(summary),
  project_get: ({ projectId }) => summary(find(projectId)),
  project_create: ({ req }) => {
    const r = req as NewProject
    const items = scan(r.files, r.kind).map((c) => media(c.fileName, [c.season, c.episode], { status: "queued", path: c.path }))
    const p = project(r.title, r.kind, r.content, items, 0)
    // Like the library: projects made while Fast Scan is on keep it.
    p.fastScan = settings.fastScan ?? false
    projects.unshift(p)
    items.forEach((m) => ingest(p, m))
    void emit("library:changed", { projectId: p.id })
    return summary(p)
  },
  project_rename: ({ projectId, title }) => {
    const p = find(projectId)
    p.title = String(title)
    void emit("library:changed", { projectId: p.id })
    return summary(p)
  },
  project_set_content: ({ projectId, content }) => {
    const p = find(projectId)
    if (p.content === "anime" || content === "anime") {
      throw { code: "invalid", message: "Anime is analysed differently from the start. Add the videos as a new anime project instead." }
    }
    p.content = content as ContentKind
    void emit("library:changed", { projectId: p.id })
    return summary(p)
  },
  project_set_fast_scan: ({ projectId, fastScan }) => {
    const p = find(projectId)
    p.fastScan = Boolean(fastScan)
    void emit("library:changed", { projectId: p.id })
    return summary(p)
  },
  project_delete: ({ projectId }) => {
    projects.splice(projects.indexOf(find(projectId)), 1)
    void emit("library:changed", { projectId: null })
    return null
  },
  import_scan: ({ paths, kind }) => scan(paths as string[], kind as ProjectKind),
  // Copies, like the real backend's answers: a change made here later must
  // reach the views as new data.
  media_list: ({ projectId }) => find(projectId).media.map((m) => ({ ...m })),
  media_get: ({ projectId, mediaId }) => ({ ...findMedia(find(projectId), mediaId) }),
  media_add: ({ projectId, files }) => {
    const p = find(projectId)
    const items = scan(files as string[], p.kind).map((c) => media(c.fileName, [c.season, c.episode], { status: "queued", path: c.path }))
    p.media.push(...items)
    refresh(p)
    items.forEach((m) => ingest(p, m))
    void emit("media:changed", { projectId: p.id, mediaId: null })
    return items.length
  },
  media_remove: ({ projectId, mediaId }) => {
    const p = find(projectId)
    // Its work stops first, as in the backend.
    cancelJobs((j) => j.mediaId === mediaId, false)
    p.media = p.media.filter((m) => m.id !== mediaId)
    refresh(p)
    void emit("media:changed", { projectId: p.id, mediaId })
    return null
  },
  media_retry: ({ projectId, mediaId }) => {
    const p = find(projectId)
    ingest(p, findMedia(p, mediaId))
    void emit("media:changed", { projectId: p.id, mediaId })
    return null
  },
  // A file whose name says "Other" plays the part of another video.
  media_relink: ({ projectId, mediaId, path, force }) => {
    const p = find(projectId)
    const m = findMedia(p, mediaId)
    const file = String(path)
    if (!force && /other/i.test(file)) return { kind: "different", reason: "It's 40:00 long instead of 52:11." }
    m.path = file
    m.fileName = file.split(/[\\/]/).pop() ?? m.fileName
    m.status = "ready"
    void emit("media:changed", { projectId: p.id, mediaId })
    void emit("library:changed", { projectId: p.id })
    return { kind: force ? "reindex" : "same" }
  },
  media_relink_folder: ({ projectId, dir }) => {
    const p = find(projectId)
    const missing = p.media.filter((m) => m.status === "missing")
    for (const m of missing) {
      m.path = `${String(dir)}\\${m.fileName}`
      m.status = "ready"
    }
    if (missing.length > 0) void emit("media:changed", { projectId: p.id, mediaId: null })
    return { same: missing.length, reindex: 0, missing: 0 }
  },
  media_indexing_still: ({ mediaId }) => {
    const j = jobs.find((x) => x.mediaId === mediaId && x.kind === "ingest" && x.state === "running" && x.frames)
    if (!j) return null
    const i = Math.min(INDEXING_STILLS.length - 1, Math.floor(((j.progress ?? 0) / 0.95) * INDEXING_STILLS.length))
    return `${FIXTURES}posters/${INDEXING_STILLS[i]}.jpg`
  },
  media_prepare_preview: () => null,
  media_set_audio: ({ projectId, mediaId, ordinal }) => {
    const m = findMedia(find(projectId), mediaId)
    m.audioOrdinal = ordinal as number | null
    void emit("media:changed", { projectId, mediaId })
    return m
  },
  media_frame_map: ({ projectId, mediaId }): FrameMap => {
    const m = findMedia(find(projectId), mediaId)
    if (m.status !== "ready") throw { code: "not_ready", message: "Still indexing." }
    return { mediaId: m.id, src: VIDEO, isProxy: m.proxy === "ready", frames: FRAMES, fps: FPS, dropFrame: false, offset: 0, step: 1 / 24, times: null }
  },
  media_segments: ({ projectId, mediaId }): Segment[] => (findMedia(find(projectId), mediaId).shots === "ready" ? SEGMENTS : []),
  media_analyze: ({ projectId, mediaId, redo }) => {
    const p = find(projectId)
    const m = findMedia(p, mediaId)
    // A running analysis restarts from the stage asked for, as in the backend.
    if (redo) cancelJobs((j) => j.kind === "analyze" && j.mediaId === mediaId, false)
    if (redo === "shots") Object.assign(m, { shots: "pending", shotCount: null })
    if (redo) Object.assign(m, { faces: "pending", faceCount: null })
    analyze(p, m)
    return null
  },
  // AniList naming: canned answers, same shapes as the real commands. The
  // project's title is the suggested search text (nothing is "sent").
  anilist_search_text: ({ projectId }) => find(projectId).title,
  anilist_search: ({ projectId, text }) => {
    anilistAllowed()
    const s = anilist(String(projectId))
    const q = String(text).toLowerCase()
    return ANILIST_SERIES.filter((m) => !s.rejected.includes(m.id) && m.title.toLowerCase().includes(q))
  },
  anilist_confirm: ({ projectId, media }) => {
    anilistAllowed()
    const s = anilist(String(projectId))
    const i = ANILIST_SERIES.findIndex((m) => m.id === media)
    const chain = i < 0 ? [] : ANILIST_SERIES.slice(i)
    if (s.media[0]?.id !== chain[0]?.id) s.suggested = null
    s.media = chain
    s.picked = false
    s.rejected = s.rejected.filter((id) => !chain.some((m) => m.id === id))
    return chain
  },
  anilist_reject: ({ projectId, media }) => {
    const s = anilist(String(projectId))
    if (!s.rejected.includes(media as number)) s.rejected.push(media as number)
    if (s.media.some((m) => m.id === media)) {
      s.media = []
      s.picked = false
      s.suggested = null
    }
    return null
  },
  anilist_suggest: ({ projectId }) => {
    anilistAllowed()
    const p = find(projectId)
    const s = anilist(String(projectId))
    if (s.media.length === 0) throw { code: "not_ready", message: "Confirm the series on AniList first." }
    const result = anilistMatch(p, s)
    if (s.auto) applyAnilist(p, s, result.names, [])
    return result
  },
  anilist_apply: ({ projectId, rev, names, merges }) => {
    anilistAllowed()
    const p = find(projectId)
    const s = anilist(String(projectId))
    if ((rev as number) !== s.rev) throw { code: "not_ready", message: "The cast changed; match the portraits again." }
    s.suggested = null
    return applyAnilist(p, s, names as AniListSuggestion[], merges as AniListMerge[])
  },
  // The look-up by itself: the first canned series (picked from the title),
  // a moment later.
  anilist_names: ({ projectId }) => {
    const p = find(projectId)
    const s = anilist(String(p.id))
    const characters = castGet(String(p.id), p.media).characters
    const wanted = settings.anilist && p.content === "anime" && characters.some((c) => !c.hidden && !c.name)
    if (wanted && !s.looked && !s.looking) {
      s.looking = true
      setTimeout(() => {
        s.looking = false
        try {
          // Switched off meanwhile: nothing is kept (and it isn't looked yet).
          if (!settings.anilist) return
          s.looked = true
          if (s.media.length === 0) {
            // The title's series, unless turned down.
            if (s.rejected.includes(ANILIST_SERIES[0].id)) return
            s.media = ANILIST_SERIES
            s.picked = true
          }
          const found = anilistMatch(p, s)
          if (s.auto && !s.picked) applyAnilist(p, s, found.names, found.merges)
          else s.suggested = { names: found.names, merges: found.merges }
        } finally {
          void emit("anilist:changed", { projectId: p.id })
        }
      }, 1500)
    }
    const open = (id: string) => characters.find((c) => c.id === id && !c.hidden)
    const names = (wanted && s.suggested?.names) || []
    const waiting = names.filter(
      (n) => open(n.characterId) && !open(n.characterId)?.name && !s.refused.includes(`${n.characterId}:${n.anilistId}`),
    )
    const merges = ((wanted && s.suggested?.merges) || []).filter((m) => open(m.from) && open(m.into))
    const any = waiting.length > 0 || merges.length > 0
    return {
      looking: s.looking,
      seriesId: any ? (s.media[0]?.id ?? null) : null,
      series: any ? (s.media[0]?.title ?? null) : null,
      names: any ? waiting : [],
      merges: any ? merges : [],
    }
  },
  // Gives only what still waits as shown, like `Project::accept_anilist_suggested`.
  anilist_accept: ({ projectId, names, merges, leftOut, leftOutMerges }) => {
    anilistAllowed()
    const p = find(projectId)
    const s = anilist(String(p.id))
    if (!s.suggested) throw { code: "not_ready", message: "These names no longer wait: the series or the cast changed." }
    const characters = castGet(String(p.id), p.media).characters
    const open = (id: string) => characters.find((c) => c.id === id && !c.hidden)
    const pair = (n: AniListSuggestion) => `${n.characterId}:${n.anilistId}`
    const waiting = s.suggested
    const isWaiting = (n: AniListSuggestion) => waiting.names.some((w) => pair(w) === pair(n))
    for (const n of leftOut as AniListSuggestion[]) if (isWaiting(n)) s.refused.push(pair(n))
    for (const m of leftOutMerges as AniListMerge[]) s.refused.push(`${m.from}:${m.anilistId}`)
    const give = (names as AniListSuggestion[]).filter(
      (n) => isWaiting(n) && open(n.characterId) && !open(n.characterId)?.name && !s.refused.includes(pair(n)),
    )
    const merge = (merges as AniListMerge[]).filter((m) =>
      waiting.merges.some((w) => w.from === m.from && w.into === m.into),
    )
    s.suggested = null
    return applyAnilist(p, s, give, merge)
  },
  anilist_dismiss: ({ projectId }) => {
    anilist(String(find(projectId).id)).suggested = null
    return null
  },
  anilist_auto: ({ projectId, on }) => {
    const s = anilist(String(projectId))
    if (on !== null && on !== undefined) s.auto = Boolean(on)
    return s.auto
  },
  cast_get: ({ projectId }) => castGet(String(projectId), find(projectId).media),
  cast_tracks: ({ projectId, filter }) => castTracks(String(find(projectId).id), filter as TrackFilter),
  cast_edit: ({ projectId, edit }) => {
    const created = castEdit(String(find(projectId).id), edit as CastEdit)
    void emit("cast:changed", { projectId })
    return created
  },
  cast_portrait: ({ projectId, trackId }) => castPortrait(String(find(projectId).id), Number(trackId)),
  media_presence: ({ projectId, mediaId }) => mediaPresence(String(find(projectId).id), String(mediaId)),
  character_shots: ({ projectId, characterId }) =>
    characterShots(String(projectId), String(characterId), find(projectId).media, SEGMENTS),
  export_presets: () => presets,
  export_default_dir: ({ projectId }) => `${settings.libraryRoot}\\Exports\\${find(projectId).title}`,
  export_start: ({ projectId, req }) => startExport(find(projectId), req as ExportRequest),
  export_plan: ({ projectId, clips }) => exportPlan(find(projectId), clips as ClipRequest[]),
  // A copy, as IPC hands one over: the same array changed in place reads as unchanged.
  exports_list: ({ projectId }) => structuredClone(find(projectId).exports),
  pack_get: ({ projectId, characterId }) => packGet(find(projectId).id, String(characterId)),
  pack_save: ({ projectId, characterId, pack }) => {
    packSave(find(projectId).id, String(characterId), pack as PackState)
    return null
  },
  pack_preview: ({ projectId, characterId, spec }) =>
    packPreview(find(projectId).id, String(characterId), spec as PackSpec, find(projectId).media, SEGMENTS),
  pack_override: ({ projectId, characterId, picks, spec }) => {
    const p = find(projectId)
    for (const { shots, include } of picks as PackPicks[]) packOverride(p.id, String(characterId), shots, include)
    return packChoices(packPreview(p.id, String(characterId), spec as PackSpec, p.media, SEGMENTS))
  },
  pack_plan: ({ projectId, characterId, spec, naming }) => {
    const n = naming as PackNaming
    const problem = templateProblem(n.template) ?? (n.merge ? fileProblem(n.file) : null)
    if (problem) throw { code: "invalid", message: problem }
    const p = find(projectId)
    const cid = String(characterId)
    const preview = packPreview(p.id, cid, spec as PackSpec, p.media, SEGMENTS)
    const names = packNames(p.id, cid, p.title, preview, n, p.media, tcName)
    const folder = n.folder && !n.merge ? characterLabel(p.id, cid) : null
    const file = n.merge ? packFileName(p.id, cid, p.title, n) : null
    return packPlan(names.slice(0, 3), preview, BYTES_PER_FRAME, folder, file)
  },
  pack_export: ({ projectId, characterId, req }) => {
    const p = find(projectId)
    const r = req as PackExportRequest
    const cid = String(characterId)
    const preview = packPreview(p.id, cid, r.spec, p.media, SEGMENTS)
    if (preview.stamp !== r.stamp) throw { code: "invalid", message: "The pack changed meanwhile. Check it and export again." }
    const taken = preview.clips.filter((c) => c.included)
    if (taken.length === 0) throw { code: "invalid", message: "There's nothing in the pack to export." }
    const problem = templateProblem(r.naming.template) ?? (r.naming.merge ? fileProblem(r.naming.file) : null)
    if (problem) throw { code: "invalid", message: problem }
    const names = packNames(p.id, cid, p.title, preview, r.naming, p.media, tcName)
    const label = characterLabel(p.id, cid)
    packSave(p.id, cid, { spec: r.spec, naming: r.naming })
    void emit("pack:changed", { projectId, characterId })
    const base = r.outDir ?? `${settings.libraryRoot}\\Exports\\${p.title}`
    return startExport(p, {
      presetId: r.presetId,
      // One file needs no folder of its own.
      outDir: r.naming.folder && !r.naming.merge ? `${base}\\${label}` : base,
      clips: taken.map((c, i) => ({ mediaId: c.shot.mediaId, start: c.start, end: c.end, name: names[i] })),
      label: `${label} · scenepack`,
      merge: r.naming.merge ? packFileName(p.id, cid, p.title, r.naming) : null,
    })
  },
  jobs_list: () => jobs,
  job_cancel: ({ jobId }) => {
    const j = jobs.find((x) => x.id === jobId)
    const running = j?.state === "running"
    cancelJobs((x) => x.id === jobId)
    // As the backend does: the export and the clips it didn't make are cancelled.
    const p = running ? projects.find((x) => x.id === j.projectId) : undefined
    const ex = p?.exports.find((x) => x.id === j?.exportId)
    if (p && ex) {
      ex.status = "cancelled"
      for (const it of ex.items) if (it.status === "queued" || it.status === "running") it.status = "cancelled"
      void emit("export:changed", { projectId: p.id, exportId: ex.id })
    }
    return null
  },
  jobs_clear_finished: () => {
    const keep = jobs.filter((j) => j.state === "running" || j.state === "queued")
    jobs.splice(0, jobs.length, ...keep)
    return null
  },
  update_check: (): UpdateStatus => ({
    state: "current",
    currentVersion: appInfo.version,
    version: null,
    notes: null,
    date: null,
    simulated: true,
  }),
  update_install: () => null,
  update_relaunch: () => false,
  open_extensions_dir: () => null,
  extensions_list: (): InstalledExtension[] => [
    {
      manifest: {
        id: "autoscene.sample.export-logger",
        name: "Export Summary Logger",
        version: "1.0.0",
        description: "Logs export details, clip counts, and output files to a summary log file.",
        author: "AutoScene Team",
        kind: "post-processor",
        entrypoint: "run.sh",
        capabilities: ["filesystem-write-export"],
        triggers: ["post-export"],
      },
      path: "/Users/you/Library/Application Support/AutoScene/extensions/export-summary-logger",
      entrypointPath: "/Users/you/Library/Application Support/AutoScene/extensions/export-summary-logger/run.sh",
    },
  ],
  // Dialog plugin: pretend the user picked something sensible.
  "plugin:dialog|open": ({ options }) => {
    const o = (options ?? {}) as { directory?: boolean; multiple?: boolean; title?: string }
    if (o.directory) return String.raw`D:\Video\Spy x Family\Season 1`
    const files = [String.raw`D:\Video\Spy x Family - 01 [1080p].mkv`, String.raw`D:\Video\Spy x Family - 02 [1080p].mkv`]
    return o.multiple ? files : files[0]
  },
}

export function installMockBackend() {
  // `?mockFail=cast_get,media_list` (before the `#`) makes those commands fail
  // the way a broken project database does, to see every error screen.
  const failing = new Set((new URLSearchParams(window.location.search).get("mockFail") ?? "").split(",").filter(Boolean))
  mockWindows("main")
  mockIPC(
    (cmd, payload) => {
      if (failing.has(cmd)) {
        throw { code: "internal", message: "AutoScene couldn't read or save its data. Details are in the log." }
      }
      const h = handlers[cmd]
      if (h) return h((payload ?? {}) as Record<string, unknown>)
      // Other built-in plugin calls (window controls…) are no-ops in the browser.
      if (cmd.startsWith("plugin:")) return null
      console.warn(`[mock] unhandled command ${cmd}`, payload)
      throw { code: "not_found", message: `Mock backend has no handler for ${cmd}` }
    },
    { shouldMockEvents: true },
  )
  // A running ingest so the activity UI has something to show.
  const f = projects[0]
  const m = f.media.find((x) => x.status === "indexing")
  if (m) window.setTimeout(() => ingest(f, m), 500)
  console.info("[mock] AutoScene mock backend installed")
}
