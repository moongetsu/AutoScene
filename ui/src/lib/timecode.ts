/**
 * SMPTE timecode helpers.
 *
 * Editors paste these straight into DaVinci/Premiere, so the frame field
 * has to count at the *nominal* rate (24 for 23.976, 30 for 29.97) rather
 * than the real one — that is what an NLE displays for the same frame.
 */

/** Nominal (integer) timecode rate for a real frame rate. */
export function nominalFps(fps: number): number {
  return Math.max(1, Math.round(fps))
}

/** True for the two rates where SMPTE drop-frame is conventional. */
export function supportsDropFrame(fps: number): boolean {
  return Math.abs(fps - 30000 / 1001) < 0.01 || Math.abs(fps - 60000 / 1001) < 0.01
}

function pad(n: number, width = 2): string {
  return String(Math.abs(n)).padStart(width, "0")
}

/**
 * Seconds → HH:MM:SS:FF (or HH:MM:SS;FF when dropFrame).
 *
 * Pass fps <= 0 to get HH:MM:SS.mmm instead — used when the source frame
 * rate could not be probed, so we never print a frame number we made up.
 */
export function formatTimecode(sec: number, fps: number, dropFrame = false): string {
  if (!Number.isFinite(sec)) sec = 0
  const sign = sec < 0 ? "-" : ""
  const t = Math.abs(sec)

  if (!Number.isFinite(fps) || fps <= 0) {
    const whole = Math.floor(t)
    const ms = Math.round((t - whole) * 1000)
    return `${sign}${pad(Math.floor(whole / 3600))}:${pad(Math.floor(whole / 60) % 60)}:${pad(
      whole % 60,
    )}.${pad(ms, 3)}`
  }

  const nominal = nominalFps(fps)
  const df = dropFrame && supportsDropFrame(fps)
  // floor, not round: the frame on screen at time t is the one whose span
  // contains t. Rounding would display the *next* frame for the back half
  // of every frame's duration. Epsilon absorbs float error when t sits
  // exactly on a boundary (e.g. 24 / 23.976 -> 23.999999996).
  let frames = Math.floor(t * fps + 1e-6)

  if (df) {
    // Standard SMPTE drop-frame renumbering: skip the first `dropCount`
    // frame *numbers* of every minute except every tenth minute.
    const dropCount = Math.round(fps * 0.066666)
    const framesPer10Min = Math.round(fps * 600)
    const framesPerMin = nominal * 60
    const tenMinBlocks = Math.floor(frames / framesPer10Min)
    const rem = frames % framesPer10Min
    frames +=
      dropCount * 9 * tenMinBlocks +
      (rem > dropCount
        ? dropCount * Math.floor((rem - dropCount) / (framesPerMin - dropCount))
        : 0)
  }

  const ff = frames % nominal
  const totalSec = Math.floor(frames / nominal)
  const ss = totalSec % 60
  const mm = Math.floor(totalSec / 60) % 60
  const hh = Math.floor(totalSec / 3600)
  return `${sign}${pad(hh)}:${pad(mm)}:${pad(ss)}${df ? ";" : ":"}${pad(ff)}`
}

/** HH:MM:SS:FF (or ;FF, or HH:MM:SS.mmm) → seconds. Returns NaN if unparseable. */
export function parseTimecode(tc: string, fps: number): number {
  const text = tc.trim()
  const msMatch = /^(-)?(\d+):(\d{1,2}):(\d{1,2})\.(\d{1,3})$/.exec(text)
  if (msMatch) {
    const [, neg, h, m, s, ms] = msMatch
    const val =
      Number(h) * 3600 + Number(m) * 60 + Number(s) + Number(ms.padEnd(3, "0")) / 1000
    return neg ? -val : val
  }

  const match = /^(-)?(\d+):(\d{1,2}):(\d{1,2})([:;])(\d{1,2})$/.exec(text)
  if (!match || !Number.isFinite(fps) || fps <= 0) return Number.NaN
  const [, neg, h, m, s, sep, f] = match
  const nominal = nominalFps(fps)
  const df = sep === ";" && supportsDropFrame(fps)

  let frames =
    ((Number(h) * 60 + Number(m)) * 60 + Number(s)) * nominal + Number(f)
  if (df) {
    const dropCount = Math.round(fps * 0.066666)
    const totalMinutes = Number(h) * 60 + Number(m)
    frames -= dropCount * (totalMinutes - Math.floor(totalMinutes / 10))
  }
  const val = frames / fps
  return neg ? -val : val
}

/** Index of the frame on screen at a given time. */
export function frameAt(sec: number, fps: number): number {
  if (!Number.isFinite(fps) || fps <= 0) return 0
  return Math.max(0, Math.floor(sec * fps + 1e-6))
}
