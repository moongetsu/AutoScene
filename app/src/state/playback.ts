import { create } from "zustand"
import { persist } from "zustand/middleware"

/** Player volume, remembered on this computer. */
type PlaybackState = {
  volume: number
  muted: boolean
  setVolume: (v: number) => void
  toggleMute: () => void
}

export const usePlayback = create<PlaybackState>()(
  persist(
    (set) => ({
      volume: 1,
      muted: false,
      setVolume: (volume) => set({ volume: Math.max(0, Math.min(1, volume)), muted: volume <= 0 }),
      toggleMute: () => set((s) => ({ muted: !s.muted })),
    }),
    { name: "autoscene.playback" },
  ),
)
