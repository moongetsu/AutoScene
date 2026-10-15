import { create } from "zustand"
import { persist } from "zustand/middleware"

type UiState = {
  sidebarCollapsed: boolean
  /** The player's details panel (clip, shot, characters, source). */
  inspectorOpen: boolean
  paletteOpen: boolean
  activityOpen: boolean
  toggleSidebar: () => void
  toggleInspector: () => void
  setPaletteOpen: (open: boolean) => void
  setActivityOpen: (open: boolean) => void
}

export const useUi = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      inspectorOpen: true,
      paletteOpen: false,
      activityOpen: false,
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
      setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
      setActivityOpen: (activityOpen) => set({ activityOpen }),
    }),
    {
      name: "autoscene.ui",
      // Only layout preferences survive a restart; overlays always start closed.
      partialize: (s) => ({ sidebarCollapsed: s.sidebarCollapsed, inspectorOpen: s.inspectorOpen }),
    },
  ),
)
