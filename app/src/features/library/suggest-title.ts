import { baseName, fileStem } from "@/lib/media"

const SEASON_DIR = /^(season|staffel|saison|temporada|series|s)\s*\d{1,2}$/i

function parentDir(p: string): string {
  return p.replace(/[\\/]+[^\\/]+[\\/]*$/, "")
}

/**
 * "[SubsPlease] Frieren - 03 (1080p)" → "Frieren"; "Breaking.Bad.S02E09" → "Breaking Bad";
 * "Dark/Season 1" → "Dark" (a season folder is named after the folder above it).
 */
export function suggestTitle(path: string, isFile: boolean): string {
  let s: string
  if (isFile) {
    const dir = parentDir(path)
    s = SEASON_DIR.test(baseName(dir)) ? baseName(parentDir(dir)) : fileStem(baseName(path))
  } else {
    s = SEASON_DIR.test(baseName(path)) ? baseName(parentDir(path)) : baseName(path)
  }
  s = s.replace(/\[[^\]]*\]|\([^)]*\)|【[^】]*】/g, " ")
  s = s.replace(/[._]/g, " ")
  s = s.replace(/\s(-\s*)?(S\d{1,2}E\d{1,4}|\d{1,2}x\d{2,3}|E\d{1,4}|\d{1,4})\b.*$/i, "")
  s = s.replace(/\b(Season|Staffel|Saison)\s*\d+\b/i, "")
  return s.replace(/\s+/g, " ").trim()
}
