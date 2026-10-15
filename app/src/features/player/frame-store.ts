import { useCallback, useSyncExternalStore } from "react"

/**
 * The frame on screen, outside React state: playback moves it every video
 * frame, and only what shows it (the timecode, the playhead, the glow)
 * draws again then. Everything else reads it when it needs it, or follows a
 * part of it that changes rarely (`useFrameSelector`: the shot under the
 * playhead).
 */
export class FrameStore {
  private frame = 0
  private readonly listeners = new Set<() => void>()

  get = (): number => this.frame

  set = (f: number) => {
    if (f === this.frame) return
    this.frame = f
    for (const l of this.listeners) l()
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }
}

/** The frame on screen; the component draws again on every frame. */
export function useFrame(store: FrameStore): number {
  return useSyncExternalStore(store.subscribe, store.get)
}

/**
 * Something derived from the frame (an index, a flag, a string); the
 * component draws again only when it changes. `select` must return equal
 * primitives for equal inputs.
 */
export function useFrameSelector<T extends string | number | boolean | null>(
  store: FrameStore,
  select: (frame: number) => T,
): T {
  const get = useCallback(() => select(store.get()), [store, select])
  return useSyncExternalStore(store.subscribe, get)
}
