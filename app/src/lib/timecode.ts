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
