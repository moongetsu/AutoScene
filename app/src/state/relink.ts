import { create } from "zustand"
import type { MediaItem } from "@/bindings"

/** A file chosen for a missing episode that doesn't look like it: asked about before it's taken. */
export type RelinkQuestion = { pid: string; media: MediaItem; path: string; reason: string }

type RelinkState = {
  question: RelinkQuestion | null
  ask: (q: RelinkQuestion) => void
  close: () => void
}

export const useRelink = create<RelinkState>((set) => ({
  question: null,
  ask: (question) => set({ question }),
  close: () => set({ question: null }),
}))
