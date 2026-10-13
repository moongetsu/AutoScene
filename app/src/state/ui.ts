import { create } from "zustand"
import { persist } from "zustand/middleware"

type UiState = {
  sidebarCollapsed: boolean
  paletteOpen: boolean
  activityOpen: boolean
  toggleSidebar: () => void
  setPaletteOpen: (open: boolean) => void
  setActivityOpen: (open: boolean) => void
}

export const useUi = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      paletteOpen: false,
      activityOpen: false,
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
      setActivityOpen: (activityOpen) => set({ activityOpen }),
    }),
    {
      name: "autoscene.ui",
      // Only layout preferences survive a restart; overlays always start closed.
      partialize: (s) => ({ sidebarCollapsed: s.sidebarCollapsed }),
    },
  ),
)
