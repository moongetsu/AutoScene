import { findFamilyByModelId, INSIGHTFACE_FAMILIES } from "@/lib/modelFamilies"

const BACKEND_LABELS: Record<string, string> = {
  insightface: "Live action / 3D",
  animecv: "Anime / cartoon",
  anime: "Anime / cartoon",
  deepface: "Lab compare",
}

const ANIME_MODEL_LABELS: Record<string, string> = {
  cel_canon: "Cel Canon",
  ink_flash: "Ink Flash",
}

const DEEPFACE_MODEL_LABELS: Record<string, string> = {
  Facenet512: "Standard",
  ArcFace: "Sharp",
  Buffalo_L: "Large",
}

const JARGON =
  /cuda|transnet|insightface|animecv|deepface|onnx|nvenc|ffmpeg|antelope|buffalo|resnet|facenet|arcface|embedding|cluster|probe|detector|gpu runtime|pyscenedetect|xmlhttprequest|\bema\b|cel embed|\bhits\b|torch|cudnn|executionprovider|ort\b/i

const STATUS_MAP: Record<string, string> = {
  cutting: "Finding scenes",
  scanning: "Scanning",
  paused: "Paused",
  ready: "Finished",
  error: "Error",
  idle: "Queued",
  exporting: "Exporting",
  queued: "Queued",
}

function stripHttpPrefix(raw: string): string {
  return raw.replace(/^HTTP\s+\d+:\s*/i, "").trim()
}

function looksTechnical(text: string): boolean {
  return (
    JARGON.test(text) ||
    /traceback|exception|stack|errno|file "|\.py\b|runtimeerror|filenotfound/i.test(
      text,
    )
  )
}

export function friendlyBackend(backend: string | undefined | null): string {
  if (!backend) return "—"
  return BACKEND_LABELS[backend.toLowerCase()] ?? backend
}

export function friendlyModel(
  backend: string | undefined | null,
  model: string | undefined | null,
): string {
  if (!model) return "—"
  const b = (backend || "").toLowerCase()

  if (b === "deepface") {
    return DEEPFACE_MODEL_LABELS[model] ?? model
  }

  if (b === "animecv" || b === "anime") {
    return ANIME_MODEL_LABELS[model] ?? model
  }

  const family = findFamilyByModelId(INSIGHTFACE_FAMILIES, model)
  if (family) {
    const level = family.levels.find((l) => l.id === model)
    if (level && family.levels.length > 1) {
      return `${family.label} · ${level.label}`
    }
    return family.label
  }

  return model
}

export function sanitizeStage(stage: string | undefined | null): string {
  if (!stage) return ""
  if (!JARGON.test(stage)) return stage

  const lower = stage.toLowerCase()
  if (lower.includes("export")) return "Exporting…"
  if (lower.includes("install") || lower.includes("download")) return "Installing…"
  if (lower.includes("cut") || lower.includes("scene")) return "Finding scene breaks…"
  if (lower.includes("probe") || lower.includes("check")) return "Preparing…"
  if (lower.includes("detect") || lower.includes("face")) return "Finding faces…"
  if (lower.includes("cluster") || lower.includes("group")) return "Grouping characters…"
  if (lower.includes("thumb") || lower.includes("portrait")) return "Building portraits…"
  if (lower.includes("model") || lower.includes("pack")) return "Preparing…"
  return "Working…"
}

/** User-facing error / toast body. Never show stacks or engine names. */
export function friendlyError(
  err: unknown,
  fallback = "Something went wrong. Try again.",
): string {
  let raw = ""
  if (err instanceof Error) raw = err.message
  else if (typeof err === "string") raw = err
  else if (err != null) raw = String(err)

  raw = stripHttpPrefix(raw)
  if (!raw) return fallback

  const lower = raw.toLowerCase()
  if (lower.includes("upload") && lower.includes("cancel")) return "Upload cancelled"
  if (lower.includes("upload failed")) return "Upload failed. Check the file and try again."
  if (lower.includes("network") || lower.includes("failed to fetch")) {
    return "Cannot reach AutoScene. Is the app running?"
  }
  if (lower.includes("missing") && lower.includes("video")) {
    return "Video file is missing. Re-import the footage."
  }
  if (
    lower.includes("zugriff verweigert") ||
    lower.includes("winerror 5") ||
    lower.includes("permission denied") ||
    lower.includes("access is denied")
  ) {
    return "Could not save project (file busy). Retry the scan."
  }
  if (lower.includes("deepface")) {
    return "DeepFace is not ready. Open Settings → Packs, install DeepFace, then restart."
  }
  if (
    lower.includes("scene") ||
    lower.includes("cut") ||
    (lower.includes("pack") && lower.includes("install"))
  ) {
    return "Scene tools are not ready. Open Settings → Packs, install what is needed, then retry."
  }
  if (lower.includes("graphics") || lower.includes("cuda") || lower.includes("gpu")) {
    return "Graphics hardware is not ready. Check Settings, then retry."
  }
  if (lower.includes("model") || lower.includes("face pack")) {
    return "Face pack is not ready. Open Settings → Packs, then retry."
  }
  if (lower.includes("export")) return "Export failed. Try again."
  if (lower.includes("delete")) return "Could not delete. Try again."
  if (lower.includes("404") || lower.includes("not found")) {
    return "That item was not found. Refresh and try again."
  }

  // Long Windows paths / WinError blobs → short fallback (not Packs).
  if (looksTechnical(raw) || raw.length > 160) {
    if (lower.includes("project.json") || lower.includes("replace")) {
      return "Could not save project (file busy). Retry the scan."
    }
    return fallback
  }
  return raw
}

export function friendlyGpuLine(
  available: boolean,
  name?: string | null,
  detail?: string,
): string {
  if (available) return name ? `Graphics · ${name}` : "Graphics ready"
  if (detail && !looksTechnical(detail) && detail.length < 80) {
    return `Graphics unavailable · ${detail}`
  }
  return "Graphics unavailable"
}

export function friendlyProjectStatus(status: string | undefined | null): string {
  if (!status) return "—"
  return STATUS_MAP[status] ?? status
}
