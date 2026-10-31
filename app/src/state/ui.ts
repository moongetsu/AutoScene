import { create } from "zustand"
import { persist } from "zustand/middleware"

export type CastViewMode = "grid" | "list"

type UiState = {
  sidebarCollapsed: boolean
  /** The player's details panel (clip, shot, characters, source). */
  inspectorOpen: boolean
  paletteOpen: boolean
  activityOpen: boolean
  castView: CastViewMode
  toggleSidebar: () => void
  toggleInspector: () => void
  setPaletteOpen: (open: boolean) => void
  setActivityOpen: (open: boolean) => void
  setCastView: (view: CastViewMode) => void
}

export const useUi = create<UiState>()(
  persist(
    (set) => ({
      sidebarCollapsed: false,
      inspectorOpen: true,
      paletteOpen: false,
      activityOpen: false,
      castView: "grid",
      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
      setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
      setActivityOpen: (activityOpen) => set({ activityOpen }),
      setCastView: (castView) => set({ castView }),
    }),
    {
      name: "autoscene.ui",
      // Only layout preferences survive a restart; overlays always start closed.
      partialize: (s) => ({
        sidebarCollapsed: s.sidebarCollapsed,
        inspectorOpen: s.inspectorOpen,
        castView: s.castView,
      }),
    },
  ),
)
