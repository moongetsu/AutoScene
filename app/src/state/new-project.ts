import { create } from "zustand"

/** The New Project dialog can be opened from the Library, the palette, or by dropping files. */
type NewProjectState = {
  open: boolean
  /** Files/folders to start with (from a drop). */
  seed: string[]
  show: (seed?: string[]) => void
  setOpen: (open: boolean) => void
}

export const useNewProject = create<NewProjectState>((set) => ({
  open: false,
  seed: [],
  show: (seed = []) => set({ open: true, seed }),
  setOpen: (open) => set(open ? { open } : { open, seed: [] }),
}))
