import { isTauri } from "@/lib/desktop"

/**
 * Update checking, plus the local memory of what the user already answered.
 *
 * The Rust side (`src-tauri/src/updater.rs`) owns the network and the install.
 * This module owns the *nagging policy* — which version was waved off, and
 * until when — because that is a UI decision and belongs next to the dialog.
 */

export type UpdateState = "available" | "current" | "unsupported"

export type UpdateStatus = {
  state: UpdateState
  currentVersion: string
  version: string | null
  notes: string | null
  date: string | null
  /** Answered from ~/.autoscene/updates.json rather than a real feed. */
  simulated: boolean
}

export type UpdateProgress = {
  downloaded: number
  total: number | null
}

/** How long "Remind Me Later" stays quiet. */
export const REMIND_LATER_MS = 24 * 60 * 60 * 1000

/** Gap between background re-checks while the app stays open. */
export const RECHECK_INTERVAL_MS = 6 * 60 * 60 * 1000

/** Let the app finish booting before the first check goes out. */
export const FIRST_CHECK_DELAY_MS = 2500

const SKIPPED_KEY = "autoscene.update.skipped"
const REMIND_AFTER_KEY = "autoscene.update.remindAfter"
const AUTO_CHECK_KEY = "autoscene.update.autoCheck"
const LAST_CHECK_KEY = "autoscene.update.lastChecked"

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, value)
  } catch {
    /* private mode / storage disabled — prompts just stop being remembered */
  }
}

/** The version the user chose "Update Later" on, if any. */
export function skippedVersion(): string | null {
  return read(SKIPPED_KEY)
}

export function skipVersion(version: string) {
  write(SKIPPED_KEY, version)
}

/** Epoch ms before which "Remind Me Later" keeps the prompt hidden. */
export function remindAfter(): number {
  const raw = read(REMIND_AFTER_KEY)
  const n = raw ? Number(raw) : 0
  return Number.isFinite(n) ? n : 0
}

export function remindIn(ms: number = REMIND_LATER_MS) {
  write(REMIND_AFTER_KEY, String(Date.now() + ms))
}

/**
 * Clear both waivers. Called when a *newer* version than the skipped one
 * appears, so waving off 0.2.0 never buries 0.3.0.
 */
export function clearDeferrals() {
  write(SKIPPED_KEY, null)
  write(REMIND_AFTER_KEY, null)
}

export function autoCheckEnabled(): boolean {
  return read(AUTO_CHECK_KEY) !== "off"
}

export function setAutoCheckEnabled(on: boolean) {
  write(AUTO_CHECK_KEY, on ? "on" : "off")
}

export function lastChecked(): number | null {
  const raw = read(LAST_CHECK_KEY)
  const n = raw ? Number(raw) : NaN
  return Number.isFinite(n) && n > 0 ? n : null
}

export function markChecked(at: number = Date.now()) {
  write(LAST_CHECK_KEY, String(at))
}

/**
 * Should the prompt open on its own for this status?
 *
 * Manual checks bypass this entirely — asking for something and being told
 * nothing is the answer would be worse than a small dialog.
 */
export function shouldPrompt(status: UpdateStatus): boolean {
  if (status.state !== "available" || !status.version) return false
  if (skippedVersion() === status.version) return false
  if (Date.now() < remindAfter()) return false
  return true
}

/**
 * A newer version than the one previously waved off resets the waivers, so a
 * single "Update Later" cannot silence the app forever.
 */
export function reconcileDeferrals(status: UpdateStatus) {
  const skipped = skippedVersion()
  if (!skipped || !status.version) return
  if (compareVersions(status.version, skipped) > 0) clearDeferrals()
}

/** Numeric-segment semver compare; unparsable segments sort as 0. */
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) =>
    v
      .replace(/^v/, "")
      .split(/[.\-+]/)
      .map((p) => {
        const n = Number(p)
        return Number.isFinite(n) ? n : 0
      })
  const left = parse(a)
  const right = parse(b)
  const len = Math.max(left.length, right.length)
  for (let i = 0; i < len; i += 1) {
    const d = (left[i] ?? 0) - (right[i] ?? 0)
    if (d !== 0) return d > 0 ? 1 : -1
  }
  return 0
}

const UNSUPPORTED: UpdateStatus = {
  state: "unsupported",
  currentVersion: "",
  version: null,
  notes: null,
  date: null,
  simulated: false,
}

/** Ask the feed. Throws with a human-readable reason when the check fails. */
export async function checkForUpdate(): Promise<UpdateStatus> {
  if (!isTauri()) return UNSUPPORTED
  const { invoke } = await import("@tauri-apps/api/core")
  return await invoke<UpdateStatus>("update_check")
}

/** Download and install. Resolves once the update is staged for restart. */
export async function installUpdate(
  onProgress?: (p: UpdateProgress) => void,
): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core")
  const { listen } = await import("@tauri-apps/api/event")

  const unlisten = onProgress
    ? await listen<UpdateProgress>("update:progress", (e) => onProgress(e.payload))
    : undefined
  try {
    await invoke("update_install")
  } finally {
    unlisten?.()
  }
}

/**
 * Restart into the new build. Resolves `false` when nothing was really
 * installed (simulate mode), in which case the app keeps running.
 */
export async function relaunchApp(): Promise<boolean> {
  const { invoke } = await import("@tauri-apps/api/core")
  return await invoke<boolean>("update_relaunch")
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}

/** "2026-08-01" → "1 Aug 2026". Anything unparsable passes through. */
export function formatReleaseDate(raw: string | null): string | null {
  if (!raw) return null
  const d = new Date(raw)
  if (Number.isNaN(d.getTime())) return raw
  return d.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  })
}
