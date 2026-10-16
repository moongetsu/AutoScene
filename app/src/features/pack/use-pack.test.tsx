import type * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { PackClip, PackPreview, PackState } from "@/bindings"
import { commands } from "@/lib/ipc"
import { queryClient } from "@/lib/queries"
import { DEFAULT_SPEC, totalsOf } from "./spec"
import { usePack } from "./use-pack"

vi.mock("@/lib/ipc", () => ({
  commands: {
    packGet: vi.fn(),
    packSave: vi.fn(() => Promise.resolve(null)),
    packPreview: vi.fn(),
    packPlan: vi.fn(() => Promise.resolve({ names: [], sizes: [], folder: null })),
    packOverride: vi.fn(),
  },
  errorMessage: (e: unknown) => String(e),
}))

const clip = (start: number, included: boolean, manual: boolean | null = null): PackClip => ({
  shot: { mediaId: "m", start, end: start + 24, sprite: null, cells: 0, trackId: 1, face: "", faceHeight: 0.3 },
  start,
  end: start + 24,
  size: "close_up",
  quality: 1,
  coverage: 1,
  others: 0,
  score: 1,
  included,
  byRules: true,
  reason: included ? null : "removed",
  manual,
})
const preview = (clips: PackClip[], stamp: string): PackPreview => ({ clips, totals: totalsOf(clips, () => 24), stamp })
const state: PackState = { spec: DEFAULT_SPEC, naming: { template: "{tc}", folder: true, merge: true, file: "{character}" } }

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
)

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.clearAllMocks()
})

describe("usePack", () => {
  it("lets nothing be exported while a click is on its way, and never flips a card back", async () => {
    vi.mocked(commands.packGet).mockResolvedValue(state)
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, true), clip(100, true)], "both"))
    let answer: () => void = () => {}
    vi.mocked(commands.packOverride).mockImplementation(() => new Promise((ok) => (answer = () => ok(null))))
    const { result } = renderHook(() => usePack("p", "c", new Map()), { wrapper })
    await waitFor(() => expect(result.current.current).toBe(true))

    // Taken out: at once on screen; no export until the backend has it.
    let click!: Promise<void>
    act(() => {
      click = result.current.setIncluded([result.current.preview.data!.clips[0]], false)
    })
    await waitFor(() => expect(result.current.preview.data?.clips[0].included).toBe(false))
    expect(result.current.current).toBe(false)

    // A refetch meanwhile (an event) brings the pack without the click: the card stays out.
    await act(() => queryClient.refetchQueries())
    expect(result.current.preview.data?.clips[0].included).toBe(false)
    expect(result.current.current).toBe(false)

    // The backend has it now: the pack it makes is what an export takes.
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, false, false), clip(100, true)], "one"))
    await act(async () => {
      answer()
      await click
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("one")
    expect(commands.packOverride).toHaveBeenCalledWith("p", "c", [{ mediaId: "m", shot: 0 }], false)
  })
})
