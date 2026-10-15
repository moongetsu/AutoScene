import { create } from "zustand"

/**
 * In/out marks per media. They live outside the player so they survive the
 * player being rebuilt — e.g. when a new preview is ready after switching the
 * audio track — and moving between episodes and back.
 */
export type Marks = { in: number | null; out: number | null }

type MarksState = {
  byMedia: Record<string, Marks>
  set: (mediaId: string, marks: Partial<Marks>) => void
}

export const NO_MARKS: Marks = { in: null, out: null }

export const useMarks = create<MarksState>((set) => ({
  byMedia: {},
  set: (mediaId, marks) =>
    set((s) => ({ byMedia: { ...s.byMedia, [mediaId]: { ...(s.byMedia[mediaId] ?? NO_MARKS), ...marks } } })),
}))
