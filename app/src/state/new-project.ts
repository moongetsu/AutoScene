import { create } from "zustand"

/** The New Project dialog can be opened from the Library, the palette, or by dropping files. */
type NewProjectState = {
  open: boolean
  /** Files/folders to start with (from a drop). */
  seed: string[]
  /** Files are being dragged over the window: the open dialog's file list is where they go. */
  dragging: boolean
  show: (seed?: string[]) => void
  setOpen: (open: boolean) => void
  setDragging: (dragging: boolean) => void
}

export const useNewProject = create<NewProjectState>((set) => ({
  open: false,
  seed: [],
  dragging: false,
  show: (seed = []) => set({ open: true, seed }),
  setOpen: (open) => set(open ? { open } : { open, seed: [] }),
  setDragging: (dragging) => set({ dragging }),
}))
