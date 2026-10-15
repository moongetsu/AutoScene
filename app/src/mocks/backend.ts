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
  seedCast,
} from "./cast"
import { packGet, packNames, packOverride, packPlan, packPreview, packSave, templateProblem } from "./pack"
import type {
  AppInfo,
  PackExportRequest,
  PackKey,
  PackNaming,
  PackSpec,
  PackState,
  AppSettings,
  ClipRequest,
  ExportItem,
  ExportPlan,
  ExportPreset,
  ExportRequest,
  ExportSummary,
  FrameMap,
  ImportCandidate,
  JobInfo,
  JobKind,
  MediaInfo,
  MediaItem,
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
  version: "1.0.0-BETA",
  platform: navigator.userAgent.includes("Mac") ? "macos" : "windows",
  arch: "x86_64",
  dataDir: String.raw`C:\Users\you\AppData\Local\AutoScene`,
  logsDir: String.raw`C:\Users\you\AppData\Local\AutoScene\logs`,
  libraryRoot: ROOT,
}

let settings: AppSettings = { libraryRoot: ROOT, exportPresetId: "prores-hq", audioLanguage: "eng" }

const presets: ExportPreset[] = [
  preset("prores-hq", "ProRes 422 HQ", "Edit-grade intra codec for After Effects, Premiere, Final Cut and Resolve. Large files.", "mov"),
  preset("dnxhr-hq", "DNxHR HQ", "Edit-grade intra codec, popular with Premiere and Resolve on Windows.", "mov"),
  preset("h264-edit", "H.264 · Editing", "Visually lossless H.264 with a short GOP for smooth scrubbing. Works everywhere.", "mp4"),
  preset("h264-mobile", "H.264 · Mobile (CapCut, Alight Motion)", "1080p H.264 with stereo sound that imports cleanly into phone editors.", "mp4"),
  preset("h264-intra", "H.264 · All-Intra (Twixtor)", "Every frame a keyframe — ideal for Twixtor and frame-by-frame work.", "mp4"),
  preset("hevc10", "HEVC 10-bit (keeps HDR)", "Smaller files that preserve 10-bit colour and HDR.", "mp4"),
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

for (const p of projects) if (p.content !== "anime") seedCast(p.id, p.media, SEGMENTS)

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

function startJob(
  kind: JobKind,
  title: string,
  p: MockProject,
  ms: number,
  opts: { mediaId?: string; exportId?: string; stage: string },
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
  jobs.unshift(job)
  void emit("job:changed", { ...job })
  const t0 = Date.now()
  const timer = window.setInterval(() => {
    if (job.state !== "running") return window.clearInterval(timer)
    const f = Math.min(1, (Date.now() - t0) / ms)
    job.progress = f
    job.etaMs = Math.max(0, ms - (Date.now() - t0))
    job.rev = ++rev
    void emit("job:progress", { id: job.id, rev: job.rev, progress: f, stage: job.stage, etaMs: job.etaMs })
    if (f >= 1) {
      window.clearInterval(timer)
      job.state = "done"
      job.finishedAt = Date.now()
      job.etaMs = null
      job.rev = ++rev
      done()
      void emit("job:changed", { ...job })
    }
  }, 250)
}

function ingest(p: MockProject, m: MediaItem) {
  m.status = "indexing"
  startJob("ingest", `Indexing ${m.label ? `${m.label} · ` : ""}${m.fileName}`, p, 6000, { mediaId: m.id, stage: "Indexing frames" }, () => {
    Object.assign(m, media(m.fileName, [m.season, m.episode], {}), { id: m.id, path: m.path, poster: null, posterSmall: null })
    m.shots = "pending"
    m.shotCount = null
    refresh(p)
    void emit("media:changed", { projectId: p.id, mediaId: m.id })
    void emit("library:changed", { projectId: p.id })
    analyze(p, m)
  })
}

function analyze(p: MockProject, m: MediaItem) {
  const facesOnly = m.shots === "ready"
  if (facesOnly) m.faces = "running"
  else m.shots = "running"
  const stage = facesOnly ? "Finding characters" : "Finding shots"
  startJob("analyze", `Analysing ${m.label ? `${m.label} · ` : ""}${m.fileName}`, p, 4000, { mediaId: m.id, stage }, () => {
    m.shots = "ready"
    m.shotCount = SHOT_COUNT
    if (p.content !== "anime") {
      m.faces = "ready"
      m.faceCount = 31
    }
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
}

function exportPlan(p: MockProject, clips: ClipRequest[]): ExportPlan {
  const frames = clips.reduce((sum, c) => sum + (c.end - c.start), 0)
  return {
    names: clips.map((c) => clipName(p, c)),
    sizes: presets.map((pr) => ({ presetId: pr.id, bytes: frames * (BYTES_PER_FRAME[pr.id] ?? 2e5) })),
    folder: null,
  }
}

function startExport(p: MockProject, req: ExportRequest): ExportSummary {
  const pr = presets.find((x) => x.id === req.presetId) ?? presets[0]
  const outDir = req.outDir ?? `${ROOT}\\Exports\\${p.title}`
  const items: ExportItem[] = req.clips.map((c, i) => {
    const name = clipName(p, c)
    return { id: i + 1, mediaId: c.mediaId, start: c.start, end: c.end, name, status: "queued", path: null, verified: false, encoder: null, bytes: null, error: null }
  })
  const ex: ExportSummary = { id: id(), projectId: p.id, createdAt: Date.now(), preset: pr, outDir, status: "running", items, label: req.label }
  p.exports.unshift(ex)
  items.forEach((it) => (it.status = "running"))
  startJob("export", items.length === 1 ? "Exporting 1 clip" : `Exporting ${items.length} clips`, p, 3500, { exportId: ex.id, stage: "Rendering" }, () => {
    for (const it of items) {
      it.status = "done"
      it.verified = true
      it.path = `${outDir}\\${it.name}.${pr.container}`
      it.encoder = pr.container === "mov" ? "prores_ks" : "h264_nvenc"
      it.bytes = (it.end - it.start) * 1.4e6
    }
    ex.status = "done"
    void emit("export:changed", { projectId: p.id, exportId: ex.id })
  })
  return ex
}

// Seed: one finished export so the Exports screen has history.
projects[0].exports.push(seedExport(projects[0]))

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
    return { path, fileName, season, episode, label, sizeBytes: 1.1e9 + Math.random() * 3e8, duplicate: false }
  })
}

// -- handlers -----------------------------------------------------------------

const handlers: Record<string, Handler> = {
  app_info: () => appInfo,
  reveal_path: () => null,
  clipboard_write: ({ text }) => {
    void navigator.clipboard?.writeText(String(text)).catch(() => {})
    return null
  },
  open_logs_dir: () => null,
  media_tools: () => ({ ready: true, version: "8.0.1-full_build-www.gyan.dev", path: String.raw`C:\ffmpeg\bin\ffmpeg.exe`, error: null }),
  settings_get: () => settings,
  settings_update: ({ settings: s }) => (settings = s as AppSettings),
  projects_list: () => [...projects].sort((a, b) => b.updatedAt - a.updatedAt).map(summary),
  project_get: ({ projectId }) => summary(find(projectId)),
  project_create: ({ req }) => {
    const r = req as NewProject
    const items = scan(r.files, r.kind).map((c) => media(c.fileName, [c.season, c.episode], { status: "queued", path: c.path }))
    const p = project(r.title, r.kind, r.content, items, 0)
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
    if (redo === "shots") Object.assign(m, { shots: "pending", shotCount: null })
    if (redo) Object.assign(m, { faces: "pending", faceCount: null })
    analyze(p, m)
    return null
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
  exports_list: ({ projectId }) => find(projectId).exports,
  pack_get: ({ projectId, characterId }) => packGet(find(projectId).id, String(characterId)),
  pack_save: ({ projectId, characterId, pack }) => {
    packSave(find(projectId).id, String(characterId), pack as PackState)
    return null
  },
  pack_preview: ({ projectId, characterId, spec }) =>
    packPreview(find(projectId).id, String(characterId), spec as PackSpec, find(projectId).media, SEGMENTS),
  pack_override: ({ projectId, characterId, shots, include }) => {
    packOverride(find(projectId).id, String(characterId), shots as PackKey[], include as boolean | null)
    void emit("pack:changed", { projectId, characterId })
    return null
  },
  pack_plan: ({ projectId, characterId, spec, naming }) => {
    const problem = templateProblem((naming as PackNaming).template)
    if (problem) throw { code: "invalid", message: problem }
    const p = find(projectId)
    const preview = packPreview(p.id, String(characterId), spec as PackSpec, p.media, SEGMENTS)
    const names = packNames(p.id, String(characterId), p.title, preview, naming as PackNaming, p.media, tcName)
    const folder = (naming as PackNaming).folder ? characterLabel(p.id, String(characterId)) : null
    return packPlan(names.slice(0, 3), preview, BYTES_PER_FRAME, folder)
  },
  pack_export: ({ projectId, characterId, req }) => {
    const p = find(projectId)
    const r = req as PackExportRequest
    const cid = String(characterId)
    const preview = packPreview(p.id, cid, r.spec, p.media, SEGMENTS)
    if (preview.stamp !== r.stamp) throw { code: "invalid", message: "The pack changed meanwhile. Check it and export again." }
    const taken = preview.clips.filter((c) => c.included)
    if (taken.length === 0) throw { code: "invalid", message: "There's nothing in the pack to export." }
    const problem = templateProblem(r.naming.template)
    if (problem) throw { code: "invalid", message: problem }
    const names = packNames(p.id, cid, p.title, preview, r.naming, p.media, tcName)
    const label = characterLabel(p.id, cid)
    packSave(p.id, cid, { spec: r.spec, naming: r.naming })
    void emit("pack:changed", { projectId, characterId })
    const base = r.outDir ?? `${settings.libraryRoot}\\Exports\\${p.title}`
    return startExport(p, {
      presetId: r.presetId,
      outDir: r.naming.folder ? `${base}\\${label}` : base,
      clips: taken.map((c, i) => ({ mediaId: c.shot.mediaId, start: c.start, end: c.end, name: names[i] })),
      label: `${label} · scenepack`,
    })
  },
  jobs_list: () => jobs,
  job_cancel: ({ jobId }) => {
    const j = jobs.find((x) => x.id === jobId)
    if (j && j.state === "running") {
      j.state = "cancelled"
      j.rev = ++rev
      j.finishedAt = Date.now()
      void emit("job:changed", { ...j })
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
