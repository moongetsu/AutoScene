/**
 * J/K/L, the way editors drive a player: L plays forward and faster on
 * each press (1×, 2×, 4×), J the same backwards, K stops. Held K with J or
 * L steps a single frame (the caller handles that).
 */
export type ShuttleKey = "j" | "k" | "l"

export const MAX_RATE = 4

/** The playback rate after a key: 0 is stopped, below 0 backwards. */
export function nextRate(rate: number, key: ShuttleKey): number {
  if (key === "k") return 0
  if (key === "l") return rate <= 0 ? 1 : Math.min(MAX_RATE, rate * 2)
  return rate >= 0 ? -1 : Math.max(-MAX_RATE, rate * 2)
}

/** "2×", "−1×": shown next to the timecode while not plain playing or stopped. */
export function rateLabel(rate: number): string | null {
  if (rate === 0 || rate === 1) return null
  return `${rate < 0 ? "−" : ""}${Math.abs(rate)}×`
}

/**
 * Frames to go back after `elapsed` seconds of reverse play at `rate` (< 0):
 * whole frames once the previous seek has landed (`ready`), the rest carried
 * over to the next tick. While a seek is still on its way the frames are
 * owed, up to a second's worth: slow seeks skip frames instead of slowing
 * the shuttle down.
 */
export function reverseFrames(
  carry: number,
  elapsed: number,
  fps: number,
  rate: number,
  ready = true,
): { frames: number; carry: number } {
  const exact = Math.min(carry + elapsed * fps * Math.abs(rate), fps * Math.abs(rate))
  if (!ready) return { frames: 0, carry: exact }
  const frames = Math.floor(exact)
  return { frames, carry: exact - frames }
}
