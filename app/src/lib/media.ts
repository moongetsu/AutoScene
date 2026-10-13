import { convertFileSrc } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-dialog"
import type { MediaInfo, MediaItem } from "@/bindings"
import { isTauri } from "./platform"

/** URL the webview can load for a local file (asset protocol). The mock backend already returns URLs. */
export function assetUrl(path: string): string {
  return isTauri ? convertFileSrc(path) : path
}

export function fileStem(name: string): string {
  const i = name.lastIndexOf(".")
  return i > 0 ? name.slice(0, i) : name
}

/** "S01E03" + file, or just the file for unnumbered media. */
export function mediaTitle(m: Pick<MediaItem, "label" | "fileName">): string {
  return m.label ? `${m.label} · ${fileStem(m.fileName)}` : fileStem(m.fileName)
}

const CODECS: Record<string, string> = {
  h264: "H.264",
  hevc: "HEVC",
  av1: "AV1",
  vp9: "VP9",
  vp8: "VP8",
  prores: "ProRes",
  dnxhd: "DNxHR",
  mpeg2video: "MPEG-2",
  mpeg4: "MPEG-4",
  vc1: "VC-1",
}

export function codecLabel(codec: string): string {
  return CODECS[codec] ?? codec.toUpperCase()
}

export function containerLabel(info: MediaInfo): string {
  const c = info.container.split(",")[0]
  return { matroska: "MKV", mov: "MP4/MOV", mpegts: "MPEG-TS", avi: "AVI", webm: "WebM" }[c] ?? c.toUpperCase()
}

/** Short format badges an editor cares about. */
export function formatFlags(m: MediaItem): string[] {
  const flags: string[] = []
  const v = m.info?.video
  if (!v) return flags
  if (v.hdr === "pq") flags.push("HDR10")
  if (v.hdr === "hlg") flags.push("HLG")
  if (v.hdr === "dolby_vision") flags.push("Dolby Vision")
  if (v.bitDepth > 8) flags.push(`${v.bitDepth}-bit`)
  if (m.vfr) flags.push("VFR")
  if (m.interlaced) flags.push("Interlaced")
  return flags
}

const LANG: Record<string, string> = {
  eng: "English",
  jpn: "Japanese",
  ger: "German",
  deu: "German",
  fre: "French",
  fra: "French",
  spa: "Spanish",
  ita: "Italian",
  kor: "Korean",
  chi: "Chinese",
  zho: "Chinese",
  por: "Portuguese",
  rus: "Russian",
}

export function audioLabel(a: MediaInfo["audio"][number]): string {
  const lang = a.language ? (LANG[a.language] ?? a.language.toUpperCase()) : null
  const ch = a.channels === 1 ? "Mono" : a.channels === 2 ? "Stereo" : a.channels === 6 ? "5.1" : a.channels === 8 ? "7.1" : `${a.channels} ch`
  return [a.title || lang || `Track ${a.ordinal + 1}`, ch].join(" · ")
}

const VIDEO_EXTENSIONS = ["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "flv", "ogv", "3gp"]

export async function pickVideoFiles(): Promise<string[]> {
  const picked = await open({ multiple: true, filters: [{ name: "Video", extensions: VIDEO_EXTENSIONS }] })
  return picked ?? []
}

export async function pickFolder(title?: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title })
  return typeof picked === "string" ? picked : null
}

/** Last path component, for titles suggested from a dropped folder. */
export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path
}
