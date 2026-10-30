import { create } from "zustand"
import { persist } from "zustand/middleware"

export type TimecodeDisplayMode = "elapsed" | "remaining" | "frames" | "percent"

type TimecodeState = {
  mode: TimecodeDisplayMode
  setMode: (mode: TimecodeDisplayMode) => void
  cycleMode: () => void
}

const MODES: TimecodeDisplayMode[] = ["elapsed", "remaining", "frames", "percent"]

export const useTimecodeMode = create<TimecodeState>()(
  persist(
    (set) => ({
      mode: "elapsed",
      setMode: (mode) => set({ mode }),
      cycleMode: () =>
        set((state) => {
          const nextIndex = (MODES.indexOf(state.mode) + 1) % MODES.length
          return { mode: MODES[nextIndex] }
        }),
    }),
    { name: "autoscene.timecode-mode" },
  ),
)

