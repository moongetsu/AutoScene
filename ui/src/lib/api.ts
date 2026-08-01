import { apiBase } from "@/lib/desktop"

let _apiToken: string | null = null
let _tokenPromise: Promise<string | null> | null = null
let _apiReady = false
let _readyWaiters: Array<() => void> = []

export function seedApiToken(token: string) {
  if (token) _apiToken = token
}

export function markApiReady() {
  _apiReady = true
  const waiters = _readyWaiters
  _readyWaiters = []
  for (const w of waiters) w()
}

export function isApiReady() {
  return _apiReady
}

/** Resolves when boot gate marks the API ready (or immediately if already). */
export function waitForApiReady(timeoutMs = 90_000): Promise<boolean> {
  if (_apiReady) return Promise.resolve(true)
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      _readyWaiters = _readyWaiters.filter((w) => w !== onReady)
      resolve(false)
    }, timeoutMs)
    const onReady = () => {
      window.clearTimeout(timer)
      resolve(true)
    }
    _readyWaiters.push(onReady)
  })
}

async function ensureApiToken(): Promise<string | null> {
  if (_apiToken) return _apiToken
  if (_tokenPromise) return _tokenPromise
  _tokenPromise = (async () => {
    try {
      const res = await fetch(`${apiBase()}/api/health`)
      if (!res.ok) return null
      const body = (await res.json()) as { token?: string }
      if (typeof body.token === "string" && body.token) {
        _apiToken = body.token
        return _apiToken
      }
    } catch {
      /* health may be down during boot */
    }
    return null
  })()
  try {
    return await _tokenPromise
  } finally {
    _tokenPromise = null
  }
}

export { ensureApiToken }

function withTokenQuery(url: string): string {
  if (!_apiToken) return url
  const join = url.includes("?") ? "&" : "?"
  return `${url}${join}token=${encodeURIComponent(_apiToken)}`
}

/** Append query params without clobbering existing ?token= (etc.). */
export function appendQuery(
  url: string,
  params: Record<string, string | number | null | undefined>,
): string {
  let out = url
  for (const [key, value] of Object.entries(params)) {
    if (value == null || value === "") continue
    const join = out.includes("?") ? "&" : "?"
    out = `${out}${join}${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`
  }
  return out
}

async function authHeaders(extra?: HeadersInit): Promise<Headers> {
  const headers = new Headers(extra)
  const token = await ensureApiToken()
  if (token) headers.set("X-AutoScene-Token", token)
  return headers
}

export type Person = {
  person_id: number
  label: string
  detection_count: number
  clip_count: number
  screen_time_sec: number
  first_sec: number
  last_sec: number
  thumbnail_path: string | null
}

export type Segment = {
  person_id: number
  start_sec: number
  end_sec: number
  /** Face-trim for preview; omit → use start/end */
  preview_start_sec?: number | null
  preview_end_sec?: number | null
  /** Timestamp of a real detection of this person (thumbnail anchor). */
  face_time_sec?: number | null
}

export type SegmentExclusion = {
  person_id: number
  start_sec: number
  end_sec: number
}

/** Shared timeline scene: all characters present in this time window. */
export type Scene = {
  scene_id: number
  start_sec: number
  end_sec: number
  person_ids: number[]
}

export type TeachCastMatch = {
  label: string
  person_id: number | null
  similarity: number
  status: "linked" | "unmatched" | "pinned"
  filename?: string | null
}

export type TeachCastResult = {
  project: Project
  matches: TeachCastMatch[]
}

export type Project = {
  id: string
  name: string
  video_path: string
  /** On-disk size of video_path (bytes); set by API from filesystem stat. */
  video_bytes?: number
  created_at: string
  updated_at: string
  status: "idle" | "cutting" | "scanning" | "paused" | "ready" | "error" | "exporting"
  progress: number
  stage: string
  error: string | null
  duration_sec: number
  progress_phase?: string
  frames_done?: number
  frames_total?: number
  frames_approx?: boolean
  detect_hits?: number
  eta_sec?: number | null
  backend: string
  model: string
  /** Persisted shot spans from pre-scan scene cut detect */
  shots?: { start_sec: number; end_sec: number }[]
  shot_count?: number
  scene_cut_detector_used?: string
  /** Live / final cut count during scene-cut phase */
  cuts_found?: number
  people: Person[]
  segments: Segment[]
  excluded_segments: SegmentExclusion[]
  /** Derived multi-cast scenes (pairwise overlap components). */
  scenes?: Scene[]
  /** Honest co-cast edges (segment overlap, not scene union). */
  cocast_edges?: { a: number; b: number; overlap_sec: number }[]
  /** True when identity data predates current pipeline; rescan recommended. */
  needs_rescan?: boolean
  identity_pipeline_version?: number
  /** Completed merged exports (persisted). */
  merged_exports?: MergedExport[]
  selected_person_id: number | null
}

export type MergedExport = {
  id: string
  person_id: number
  person_label: string
  output_path: string
  scene_count: number
  finished_at: string
  export_mode?: string
  video_codec?: string
}

export type ExportQueueJob = {
  id: string
  project_id: string
  person_id: number
  person_label: string
  output_path: string
  export_mode: string
  video_codec: string
  scene_count: number
  status: "queued" | "running" | "done" | "failed" | "cancelled"
  progress: number
  stage: string
  error?: string | null
}

export type ExportQueueSnapshot = {
  active: boolean
  current: ExportQueueJob | null
  pending: ExportQueueJob[]
  recent: ExportQueueJob[]
  pending_count: number
  global_busy?: boolean
}

export type GpuInfo = {
  available: boolean
  name: string | null
  provider: string
  detail: string
}

export type FaceSelectMode = "all" | "largest" | "relative" | "top_n"

export type ContentPresetId = "live_action" | "animation" | "anime" | ""

export type ContentPresetInfo = {
  id: Exclude<ContentPresetId, "">
  label: string
  blurb: string
  cluster_merge_sim: number
  cluster_min_size: number
  min_face_size: number
  det_size: number
  default_backend: string
  default_model: string
}

export type AnimeCVModelInfo = {
  id: string
  label: string
  tagline: string
  blurb: string
  role: string
  installed?: boolean
}

export type ModelVaultStatus = {
  deps: {
    id: string
    label: string
    tagline: string
    installed: boolean
    detail: string
  }[]
  packs: {
    id: string
    label: string
    tagline: string
    blurb: string
    role: string
    family: string
    installed: boolean
    path: string
    approx_mb?: number
  }[]
  animecv_models: AnimeCVModelInfo[]
  ready_count: number
  total_count: number
  all_ready: boolean
  root: string
  job: {
    id: string
    status: string
    stage: string
    progress: number
    error: string | null
    log: string[]
  }
}

export type SceneCutDetector = "adaptive" | "transnet" | "auto"

export type AppSettings = {
  sample_fps: number
  det_size: number
  /** Longest side before detect; 0 = native */
  detect_max_edge: number
  min_face_size: number
  cluster_min_size: number
  cluster_merge_sim: number
  content_preset: ContentPresetId
  content_presets?: ContentPresetInfo[]
  animecv_models?: AnimeCVModelInfo[]
  prefetch_depth: number
  face_select_mode: FaceSelectMode
  face_relative_pct: number
  face_top_n: number
  default_backend: string
  default_model: string
  scene_cut_detector: SceneCutDetector
  scene_cut_adaptive_threshold: number
  scene_cut_transnet_threshold: number
  scene_cut_capabilities?: {
    adaptive: boolean
    transnet: boolean
    modes: SceneCutDetector[]
  }
  data_dir: string
}

/** Shipped defaults — used to fill gaps and to power "Reset" in Settings. */
export const SETTINGS_DEFAULTS = {
  sample_fps: 12,
  det_size: 1280,
  detect_max_edge: 1920,
  min_face_size: 40,
  cluster_min_size: 8,
  cluster_merge_sim: 0.55,
  prefetch_depth: 8,
  face_select_mode: "all" as FaceSelectMode,
  face_relative_pct: 40,
  face_top_n: 2,
  default_backend: "insightface",
  default_model: "buffalo_l",
  scene_cut_detector: "auto" as SceneCutDetector,
  scene_cut_adaptive_threshold: 3,
  scene_cut_transnet_threshold: 0.5,
} as const

export function normalizeSettings(
  raw: Partial<AppSettings> & { data_dir: string },
): AppSettings {
  const mode = raw.face_select_mode
  const face_select_mode: FaceSelectMode =
    mode === "largest" || mode === "relative" || mode === "top_n" || mode === "all"
      ? mode
      : "all"
  const presetRaw = (raw.content_preset || "").toLowerCase()
  const content_preset: ContentPresetId =
    presetRaw === "live_action" || presetRaw === "animation" || presetRaw === "anime"
      ? presetRaw
      : ""
  const cutRaw = (raw.scene_cut_detector || "auto").toLowerCase()
  const scene_cut_detector: SceneCutDetector =
    cutRaw === "adaptive" || cutRaw === "transnet" || cutRaw === "auto"
      ? cutRaw
      : "auto"
  return {
    sample_fps: raw.sample_fps ?? SETTINGS_DEFAULTS.sample_fps,
    det_size: raw.det_size ?? SETTINGS_DEFAULTS.det_size,
    detect_max_edge: raw.detect_max_edge ?? SETTINGS_DEFAULTS.detect_max_edge,
    min_face_size: raw.min_face_size ?? SETTINGS_DEFAULTS.min_face_size,
    cluster_min_size: raw.cluster_min_size ?? SETTINGS_DEFAULTS.cluster_min_size,
    cluster_merge_sim: raw.cluster_merge_sim ?? SETTINGS_DEFAULTS.cluster_merge_sim,
    content_preset,
    content_presets: raw.content_presets,
    animecv_models: raw.animecv_models?.length
      ? raw.animecv_models
      : ANIMECV_MODEL_FALLBACK,
    prefetch_depth: raw.prefetch_depth ?? SETTINGS_DEFAULTS.prefetch_depth,
    face_select_mode,
    face_relative_pct: raw.face_relative_pct ?? SETTINGS_DEFAULTS.face_relative_pct,
    face_top_n: raw.face_top_n ?? SETTINGS_DEFAULTS.face_top_n,
    default_backend: raw.default_backend ?? SETTINGS_DEFAULTS.default_backend,
    default_model: raw.default_model ?? SETTINGS_DEFAULTS.default_model,
    scene_cut_detector,
    scene_cut_adaptive_threshold:
      raw.scene_cut_adaptive_threshold ?? SETTINGS_DEFAULTS.scene_cut_adaptive_threshold,
    scene_cut_transnet_threshold:
      raw.scene_cut_transnet_threshold ?? SETTINGS_DEFAULTS.scene_cut_transnet_threshold,
    scene_cut_capabilities: raw.scene_cut_capabilities,
    data_dir: raw.data_dir,
  }
}

export const ANIMECV_MODEL_FALLBACK: AnimeCVModelInfo[] = [
  {
    id: "cel_canon",
    label: "Cel Canon",
    tagline: "Higher quality",
    blurb: "Best accuracy, slower.",
    role: "quality",
  },
  {
    id: "ink_flash",
    label: "Ink Flash",
    tagline: "Faster",
    blurb: "Faster, slightly less accurate.",
    role: "speed",
  },
]

/** Fallback catalog if API is older than preset support */
export const CONTENT_PRESET_FALLBACK: ContentPresetInfo[] = [
  {
    id: "live_action",
    label: "Live-action",
    blurb: "Best for real people.",
    cluster_merge_sim: 0.55,
    cluster_min_size: 8,
    min_face_size: 40,
    det_size: 1280,
    default_backend: "insightface",
    default_model: "buffalo_l",
  },
  {
    id: "animation",
    label: "3D / stylized",
    blurb: "CGI and stylized characters (balanced gates).",
    cluster_merge_sim: 0.58,
    cluster_min_size: 6,
    min_face_size: 40,
    det_size: 1280,
    default_backend: "insightface",
    default_model: "buffalo_l",
  },
  {
    id: "anime",
    label: "Anime / cartoon",
    blurb: "2D anime and cartoons.",
    cluster_merge_sim: 0.58,
    cluster_min_size: 5,
    min_face_size: 36,
    det_size: 1280,
    default_backend: "animecv",
    default_model: "cel_canon",
  },
]

export type UploadProgressEvent = {
  loaded: number
  total: number
  percent: number
}

function uploadMultipart(
  path: string,
  fd: FormData,
  onProgress?: (event: UploadProgressEvent) => void,
): Promise<Project> {
  return new Promise((resolve, reject) => {
    void (async () => {
      const xhr = new XMLHttpRequest()
      xhr.open("POST", `${apiBase()}${path}`)
      const token = await ensureApiToken()
      if (token) xhr.setRequestHeader("X-AutoScene-Token", token)

      xhr.upload.addEventListener("progress", (event) => {
        if (!onProgress || !event.lengthComputable) return
        onProgress({
          loaded: event.loaded,
          total: event.total,
          percent: (event.loaded / event.total) * 100,
        })
      })

      xhr.addEventListener("load", () => {
        const text = xhr.responseText
        if (xhr.status >= 200 && xhr.status < 300) {
          try {
            resolve(JSON.parse(text) as Project)
          } catch {
            reject(new Error("Invalid upload response"))
          }
          return
        }
        let detail = text || xhr.statusText || "Upload failed"
        try {
          const parsed = JSON.parse(text) as { detail?: unknown }
          if (typeof parsed?.detail === "string") detail = parsed.detail
        } catch {
          /* plain */
        }
        reject(new Error(detail))
      })

      xhr.addEventListener("error", () => reject(new Error("Upload failed")))
      xhr.addEventListener("abort", () => reject(new Error("Upload cancelled")))
      xhr.send(fd)
    })().catch(reject)
  })
}

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = await authHeaders(init?.headers)
  let res: Response
  try {
    res = await fetch(`${apiBase()}${path}`, { ...init, headers })
  } catch {
    throw new Error("Cannot reach AutoScene. Is the app running?")
  }
  if (!res.ok) {
    const text = (await res.text()).trim()
    let detail = text || res.statusText || "Request failed"
    try {
      const parsed = JSON.parse(text) as { detail?: unknown }
      if (typeof parsed?.detail === "string") detail = parsed.detail
    } catch {
      /* plain text body */
    }
    throw new Error(detail || res.statusText || "Request failed")
  }
  return res.json() as Promise<T>
}

export const api = {
  health: () =>
    fetch(`${apiBase()}/api/health`).then(async (res) => {
      const body = (await res.json()) as { ok: boolean; token?: string; version?: string }
      if (typeof body.token === "string" && body.token) _apiToken = body.token
      if (!res.ok) throw new Error("Health check failed")
      return body
    }),
  gpu: () => req<GpuInfo>("/api/gpu"),
  settings: () => req<AppSettings>("/api/settings"),
  updateSettings: (body: Partial<Omit<AppSettings, "data_dir" | "animecv_models" | "content_presets">>) =>
    req<AppSettings>("/api/settings", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  modelsStatus: () => req<ModelVaultStatus>("/api/models/status"),
  modelsInstall: (targets: string[] = ["all"]) =>
    req<ModelVaultStatus>("/api/models/install", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ targets }),
    }),
  modelsInstallStatus: () => req<ModelVaultStatus>("/api/models/install/status"),
  projects: () => req<Project[]>("/api/projects"),
  project: (id: string) => req<Project>(`/api/projects/${id}`),
  createPath: (body: {
    name: string
    video_path: string
    backend?: string
    model?: string
  }) =>
    req<Project>("/api/projects", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  upload: async (
    file: File,
    opts: {
      name?: string
      backend?: string
      model?: string
      autostart?: boolean
      onProgress?: (event: UploadProgressEvent) => void
    } = {},
  ) => {
    const fd = new FormData()
    fd.append("file", file)
    fd.append("name", opts.name || file.name.replace(/\.[^.]+$/, ""))
    fd.append("backend", opts.backend || "insightface")
    fd.append("model", opts.model || "antelopev2")
    fd.append("autostart", String(opts.autostart ?? true))
    return uploadMultipart("/api/projects/upload", fd, opts.onProgress)
  },
  scan: (id: string) =>
    req<Project>(`/api/projects/${id}/scan`, { method: "POST" }),
  scanDebug: (id: string, lines = 200) =>
    req<{
      project_id: string
      status: string
      stage: string
      backend: string
      model: string
      frames_done: number
      frames_total: number
      detect_hits: number
      error: string | null
      runtime_settings: Record<string, unknown>
      log_path: string
      exists: boolean
      lines: string[]
      total_lines?: number
    }>(`/api/projects/${id}/scan-debug?lines=${lines}`),
  pauseScan: (id: string) =>
    req<Project>(`/api/projects/${id}/scan/pause`, { method: "POST" }),
  resumeScan: (id: string) =>
    req<Project>(`/api/projects/${id}/scan/resume`, { method: "POST" }),
  stopScan: (id: string) =>
    req<Project>(`/api/projects/${id}/scan/stop`, { method: "POST" }),
  deleteProject: (id: string, opts: { confirmStop?: boolean } = {}) => {
    const q = opts.confirmStop ? "?confirm_stop=true" : ""
    return req<{ ok: boolean; id: string; stopped_scan: boolean }>(
      `/api/projects/${id}${q}`,
      { method: "DELETE" },
    )
  },
  renamePerson: (projectId: string, personId: number, label: string) =>
    req<Project>(`/api/projects/${projectId}/people/${personId}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ label }),
    }),
  teachCast: async (
    projectId: string,
    files: File[],
    opts: {
      mode: "pin" | "match" | "pack"
      label?: string
      personId?: number
    },
  ) => {
    const fd = new FormData()
    fd.append("mode", opts.mode)
    if (opts.label) fd.append("label", opts.label)
    if (opts.personId != null) fd.append("person_id", String(opts.personId))
    for (const file of files) {
      fd.append("files", file, file.name)
    }
    return req<TeachCastResult>(`/api/projects/${projectId}/cast/teach`, {
      method: "POST",
      body: fd,
    })
  },
  mergePeople: (
    projectId: string,
    body: { keep_person_id: number; merge_person_ids: number[] },
  ) =>
    req<Project>(`/api/projects/${projectId}/people/merge`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  toggleSegmentExclusion: (
    projectId: string,
    body: {
      person_id: number
      start_sec: number
      end_sec: number
      excluded: boolean
    },
  ) =>
    req<Project>(`/api/projects/${projectId}/segments/exclusion`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  exportMerged: (
    projectId: string,
    body: {
      person_id: number
      output_path: string
      reencode?: boolean
      export_mode?: "copy" | "quality" | "lossless"
      video_codec?:
        | "h264_nvenc"
        | "hevc_nvenc"
        | "av1_nvenc"
        | "h264_videotoolbox"
        | "hevc_videotoolbox"
        | "auto"
    },
  ) =>
    req<{
      ok: boolean
      queued: boolean
      job_id: string
      output_path: string
      scene_count: number
      person_label: string
      queue?: ExportQueueSnapshot
    }>(`/api/projects/${projectId}/export/merged`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  exportQueue: (projectId: string) =>
    req<ExportQueueSnapshot>(`/api/projects/${projectId}/export/queue`),
  exportQueueCancel: (projectId: string) =>
    req<{ ok: boolean; cancelled_ids: string[]; queue?: ExportQueueSnapshot }>(
      `/api/projects/${projectId}/export/queue/cancel`,
      { method: "POST" },
    ),
  exportCapabilities: () =>
    req<{
      codecs: Record<string, boolean>
      modes: string[]
      default_mode: string
      default_codec: string
    }>("/api/export/capabilities"),
  exportPerson: (projectId: string, personId: number, reencode = false) =>
    req<{ ok: boolean; out_dir: string; count: number; clips: string[] }>(
      `/api/projects/${projectId}/export`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ person_id: personId, reencode }),
      },
    ),
  openFolder: (path: string) =>
    req<{ ok: boolean }>("/api/system/open-folder", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    }),
  videoUrl: (projectId: string) =>
    withTokenQuery(`${apiBase()}/api/projects/${projectId}/video`),
  /** Cached JPEG still for Export tiles (t in seconds).
   * Pass personId for a focus-person face crop when detection geometry exists. */
  sceneFrameUrl: (
    projectId: string,
    timeSec: number,
    personId?: number | null,
  ) => {
    const t = encodeURIComponent(String(Math.max(0, timeSec)))
    const pid =
      personId != null && Number.isFinite(personId)
        ? `&person_id=${encodeURIComponent(String(personId))}`
        : ""
    return withTokenQuery(
      `${apiBase()}/api/projects/${projectId}/scene-frame?t=${t}${pid}`,
    )
  },
  videoProxyStatus: (projectId: string) =>
    req<{
      source_suffix: string
      source_video_codec?: string | null
      source_audio_codec?: string | null
      native_ok: boolean
      proxy_ready: boolean
      proxy_path: string | null
      needs_transcode?: boolean
    }>(`/api/projects/${projectId}/video-proxy-status`),
  videoMeta: (projectId: string) =>
    req<{ video_path: string; video_bytes: number; exists: boolean }>(
      `/api/projects/${projectId}/video-meta`,
    ),
  mediaUrl: (projectId: string, rel: string) =>
    withTokenQuery(`${apiBase()}/api/projects/${projectId}/media/${rel}`),
  /** Cache-busted latest detect crop for Import tracker. */
  liveFaceUrl: (projectId: string, rev: number | string) =>
    withTokenQuery(`${apiBase()}/api/projects/${projectId}/live-face?v=${rev}`),
}

export function formatDuration(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "0:00"
  const s = Math.round(sec)
  const m = Math.floor(s / 60)
  const r = s % 60
  const h = Math.floor(m / 60)
  if (h > 0) return `${h}:${String(m % 60).padStart(2, "0")}:${String(r).padStart(2, "0")}`
  return `${m}:${String(r).padStart(2, "0")}`
}

export function isSegmentExcluded(
  exclusions: SegmentExclusion[],
  segment: Segment,
): boolean {
  const key = (s: SegmentExclusion | Segment) =>
    `${s.person_id}:${s.start_sec.toFixed(4)}:${s.end_sec.toFixed(4)}`
  const target = key(segment)
  return exclusions.some((ex) => key(ex) === target)
}
