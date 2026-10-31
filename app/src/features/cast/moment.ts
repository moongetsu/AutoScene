import type { MediaItem } from "@/bindings"
import type { FrameClock } from "@/lib/frame-clock"

/**
 * A face's moment can play right away: its episode is indexed and plays as
 * it is, or from a preview copy that is done. Asking for any other episode's
 * frame map would queue a preview copy (minutes of work) for a face the
 * pointer only passed over; those faces keep their still.
 */
export function canPlayMoment(m: Pick<MediaItem, "status" | "proxy"> | undefined): boolean {
  return m?.status === "ready" && (m.proxy === "not_needed" || m.proxy === "ready")
}

/**
 * A track's frames `start..end` (the end not in it) as a loop: the player
 * time it starts from (the middle of its first frame, so the seek can't land
 * on the one before) and its first and last frame. Null when none of its
 * frames is in the episode.
 */
export type MomentLoop = { from: number; first: number; last: number }

export function momentLoop(clock: FrameClock, start: number, end: number): MomentLoop | null {
  const first = Math.max(0, Math.floor(start))
  const last = Math.min(Math.ceil(end), clock.frames) - 1
  if (last < first) return null
  return { from: clock.seekTime(first), first, last }
}

/**
 * The frame shown at player time `t` is the moment's last (or past it): go
 * back to its start now, before the next shot can flash by. A one-frame
 * moment stays still instead.
 */
export function loopsBack(clock: FrameClock, loop: MomentLoop, t: number): boolean {
  return loop.last > loop.first && clock.frameAt(t) >= loop.last
}
