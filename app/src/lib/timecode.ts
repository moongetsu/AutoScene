/** Frame rate as an exact fraction (e.g. 24000/1001). */
export type Fps = { num: number; den: number }

export function fpsValue(fps: Fps): number {
  return fps.num / fps.den
}

/** "23.976", "25", "29.97" — how editors say frame rates. */
export function formatFps(fps: Fps): string {
  const v = fpsValue(fps)
  const rounded = Math.round(v * 1000) / 1000
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(3).replace(/0+$/, "")
}

/** SMPTE drop-frame applies to 29.97 and 59.94 only. */
export function isDropFrameRate(fps: Fps): boolean {
  const base = Math.round(fpsValue(fps))
  return fps.den === 1001 && (base === 30 || base === 60)
}

/**
 * Timecode of frame `f` counted at the nominal rate, exactly like the Rust
 * side (`as_core::Timecode::from_frame`), so the UI, file names and sidecars
 * always agree. Drop-frame uses `;` before the frames field.
 */
export function timecode(f: number, fps: Fps, dropFrame: boolean): string {
  const base = Math.max(1, Math.round(fpsValue(fps)))
  const df = dropFrame && isDropFrameRate(fps)
  let n = Math.max(0, Math.floor(f))
  if (df) {
    const drop = base / 15
    const per10 = base * 600 - drop * 9
    const perMin = base * 60 - drop
    const tens = Math.floor(n / per10)
    const rem = n % per10
    n += drop * 9 * tens + (rem > drop ? drop * Math.floor((rem - drop) / perMin) : 0)
  }
  const hh = Math.floor(n / (base * 3600))
  const mm = Math.floor(n / (base * 60)) % 60
  const ss = Math.floor(n / base) % 60
  const ff = n % base
  const p = (v: number) => String(v).padStart(2, "0")
  return `${p(hh)}:${p(mm)}:${p(ss)}${df ? ";" : ":"}${p(ff)}`
}

/**
 * Frame for what someone typed into the "go to" field:
 * - a timecode, right-aligned (`1:02:03:04`, `02:03:04` = mm:ss:ff, `;` or `.` also work),
 * - a plain frame number (`17099`),
 * - a relative jump in frames (`+48`, `-12`).
 * Drop-frame labels are converted back exactly (the inverse of `timecode`).
 * Returns `null` for anything else.
 */
export function parseTimecode(text: string, fps: Fps, dropFrame: boolean, current = 0): number | null {
  const t = text.trim()
  if (/^[+-]\d+$/.test(t)) return current + Number(t)
  if (/^\d+$/.test(t)) return Number(t)
  if (!/^\d{1,2}([:;.]\d{1,2}){1,3}$/.test(t)) return null
  const parts = t.split(/[:;.]/).map(Number)
  while (parts.length < 4) parts.unshift(0)
  const [hh, mm, ss, ff] = parts
  const base = Math.max(1, Math.round(fpsValue(fps)))
  if (mm > 59 || ss > 59 || ff >= base) return null
  let n = ((hh * 60 + mm) * 60 + ss) * base + ff
  if (dropFrame && isDropFrameRate(fps)) {
    const drop = base / 15
    const minutes = hh * 60 + mm
    // Labels 00 and 01 (00-03 at 59.94) don't exist in minutes not divisible by ten.
    if (mm % 10 !== 0 && ss === 0 && ff < drop) n += drop - ff
    n -= drop * (minutes - Math.floor(minutes / 10))
  }
  return n
}
