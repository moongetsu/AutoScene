import type * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { JobInfo, PackChoices, PackClip, PackPreview, PackState } from "@/bindings"
import { commands } from "@/lib/ipc"
import { queryClient } from "@/lib/queries"
import { DEFAULT_SPEC, totalsOf } from "./spec"
import { useActivity } from "@/state/activity"
import { packExportStarted, usePack, usePackExport } from "./use-pack"

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

const clip = (start: number, included: boolean, manual: boolean | null = null, byRules = true): PackClip => ({
  shot: { mediaId: "m", start, end: start + 24, sprite: null, cells: 0, trackId: 1, face: "", faceHeight: 0.3 },
  start,
  end: start + 24,
  size: "close_up",
  quality: 1,
  coverage: 1,
  others: 0,
  score: 1,
  included,
  byRules,
  reason: included ? null : manual === false ? "removed" : "short",
  manual,
})
/** The picks of one click, as the backend gets them. */
const picks = (...groups: [number[], boolean | null][]) =>
  groups.map(([starts, include]) => ({ shots: starts.map((shot) => ({ mediaId: "m", shot })), include }))
const preview = (clips: PackClip[], stamp: string, shape = "two shots"): PackPreview => ({
  clips,
  totals: totalsOf(clips, () => 24),
  stamp,
  shape,
})
/** The backend's answer to a pick: the choices of `p`. */
const choicesOf = (p: PackPreview): PackChoices => ({
  choices: p.clips.map(({ included, byRules, reason, manual }) => ({ included, byRules, reason, manual })),
  totals: p.totals,
  stamp: p.stamp,
  shape: p.shape,
})
const state: PackState = { spec: DEFAULT_SPEC, naming: { template: "{tc}", folder: true, merge: true, file: "{character}" } }

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
)

/** The hook over a pack of two shots, both in, once it shows them. */
async function mounted() {
  vi.mocked(commands.packGet).mockResolvedValue(state)
  vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, true), clip(100, true)], "both"))
  const hook = renderHook(() => usePack("p", "c", new Map()), { wrapper })
  await waitFor(() => expect(hook.result.current.current).toBe(true))
  return hook
}

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.clearAllMocks()
})

describe("usePack", () => {
  it("lets nothing be exported while a click is on its way, and never flips a card back", async () => {
    const { result } = await mounted()
    let answer: (a: PackChoices) => void = () => {}
    vi.mocked(commands.packOverride).mockImplementation(() => new Promise((ok) => (answer = ok)))

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

    // The backend has it now: its answer is what an export takes.
    await act(async () => {
      answer(choicesOf(preview([clip(0, false, false), clip(100, true)], "one")))
      await click
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("one")
    expect(result.current.preview.data?.clips[0]).toMatchObject({ included: false, manual: false })
    expect(commands.packOverride).toHaveBeenCalledWith("p", "c", picks([[0], false]), DEFAULT_SPEC)
  })

  it("fetches only the choices a click changed, and the plan once the clicks are in", async () => {
    const { result } = await mounted()
    const shots = vi.mocked(commands.packPreview).mock.calls.length
    const plans = vi.mocked(commands.packPlan).mock.calls.length
    vi.mocked(commands.packOverride).mockResolvedValue(choicesOf(preview([clip(0, false, false), clip(100, true)], "one")))
    await act(() => result.current.setIncluded([result.current.preview.data!.clips[0]], false))
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("one")
    // The shots aren't fetched again; the plan (names, size) is.
    expect(vi.mocked(commands.packPreview).mock.calls.length).toBe(shots)
    await waitFor(() => expect(vi.mocked(commands.packPlan).mock.calls.length).toBe(plans + 1))
  })

  it("fetches the whole pack when it changed otherwise meanwhile", async () => {
    const { result } = await mounted()
    // An analysis found a third shot meanwhile: the answer is of another pack.
    const now = preview([clip(0, false, false), clip(50, true), clip(100, true)], "three", "three shots")
    vi.mocked(commands.packOverride).mockResolvedValue(choicesOf(now))
    vi.mocked(commands.packPreview).mockResolvedValue(now)
    await act(() => result.current.setIncluded([result.current.preview.data!.clips[0]], false))
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.clips.map((c) => c.start)).toEqual([0, 50, 100])
    expect(result.current.preview.data?.stamp).toBe("three")
  })

  it("fetches the whole pack again when a fetch of it is on its way as the answer comes", async () => {
    const { result } = await mounted()
    let answer: (a: PackChoices) => void = () => {}
    vi.mocked(commands.packOverride).mockImplementation(() => new Promise((ok) => (answer = ok)))
    let click!: Promise<void>
    act(() => {
      click = result.current.setIncluded([result.current.preview.data!.clips[0]], false)
    })
    await waitFor(() => expect(commands.packOverride).toHaveBeenCalled())
    // An event asks for the whole pack meanwhile; it comes late, made before the pick.
    let late: (p: PackPreview) => void = () => {}
    vi.mocked(commands.packPreview).mockImplementationOnce(() => new Promise((ok) => (late = ok)))
    act(() => void queryClient.invalidateQueries())
    const after = preview([clip(0, false, false), clip(100, true)], "one")
    vi.mocked(commands.packPreview).mockResolvedValue(after)
    await act(async () => {
      answer(choicesOf(after))
      await click
      late(preview([clip(0, true), clip(100, true)], "both"))
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("one")
    expect(result.current.preview.data?.clips[0].included).toBe(false)
  })

  it("refetches the pack of filters changed during a click from the start once the click is in", async () => {
    const { result } = await mounted()
    let answer: (a: PackChoices) => void = () => {}
    vi.mocked(commands.packOverride).mockImplementation(() => new Promise((ok) => (answer = ok)))
    let click!: Promise<void>
    act(() => {
      click = result.current.setIncluded([result.current.preview.data!.clips[0]], false)
    })
    await waitFor(() => expect(commands.packOverride).toHaveBeenCalled())
    // Other filters meanwhile: their pack's first fetch reads it before the click.
    const solo = { ...DEFAULT_SPEC, solo: true }
    let late: (p: PackPreview) => void = () => {}
    vi.mocked(commands.packPreview).mockImplementationOnce(() => new Promise((ok) => (late = ok)))
    act(() => result.current.setSpec(solo))
    await waitFor(() => expect(commands.packPreview).toHaveBeenCalledWith("p", "c", solo))
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, false, false), clip(100, true)], "solo"))
    await act(async () => {
      answer(choicesOf(preview([clip(0, false, false), clip(100, true)], "one")))
      await click
      late(preview([clip(0, true), clip(100, true)], "before the click"))
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("solo")
    expect(result.current.preview.data?.clips[0].included).toBe(false)
  })

  it("sends a click's picks and hand-backs at once, and keeps later clicks on screen until they are in", async () => {
    vi.mocked(commands.packGet).mockResolvedValue(state)
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, false, false), clip(100, false, null, false)], "none"))
    const { result } = renderHook(() => usePack("p", "c", new Map()), { wrapper })
    await waitFor(() => expect(result.current.current).toBe(true))
    const answers: ((a: PackChoices) => void)[] = []
    vi.mocked(commands.packOverride).mockImplementation(() => new Promise((ok) => answers.push(ok)))

    // Both in: one taken out by hand goes back to the filters, the other is picked.
    let first!: Promise<void>
    act(() => {
      first = result.current.setIncluded(result.current.preview.data!.clips, true)
    })
    await waitFor(() => expect(answers.length).toBe(1))
    expect(commands.packOverride).toHaveBeenCalledWith("p", "c", picks([[100], true], [[0], null]), DEFAULT_SPEC)
    // Out again, before the first click is in: it waits for it.
    let second!: Promise<void>
    act(() => {
      second = result.current.setIncluded([result.current.preview.data!.clips[0]], false)
    })
    await act(async () => {
      answers[0](choicesOf(preview([clip(0, true), clip(100, true, true, false)], "both")))
      await first
    })
    await waitFor(() => expect(answers.length).toBe(2))
    // The first is in; the second still shows on screen, and nothing can be exported.
    expect(result.current.preview.data?.clips.map((c) => c.included)).toEqual([false, true])
    expect(result.current.current).toBe(false)
    await act(async () => {
      answers[1](choicesOf(preview([clip(0, false, false), clip(100, true, true, false)], "one")))
      await second
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(result.current.preview.data?.stamp).toBe("one")
    expect(result.current.preview.data?.clips.map((c) => c.manual)).toEqual([false, true])
  })

  it("shows the pack as the backend has it when a click fails", async () => {
    const { result } = await mounted()
    const shots = vi.mocked(commands.packPreview).mock.calls.length
    vi.mocked(commands.packOverride).mockRejectedValue("This character isn't in the cast any more.")
    await act(() => result.current.setIncluded([result.current.preview.data!.clips[0]], false))
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(vi.mocked(commands.packPreview).mock.calls.length).toBe(shots + 1)
    expect(result.current.preview.data?.clips[0].included).toBe(true)
  })

  it("hands picks back to the filters the same way", async () => {
    vi.mocked(commands.packGet).mockResolvedValue(state)
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, false, false), clip(100, true)], "one"))
    const { result } = renderHook(() => usePack("p", "c", new Map()), { wrapper })
    await waitFor(() => expect(result.current.current).toBe(true))
    vi.mocked(commands.packOverride).mockResolvedValue(choicesOf(preview([clip(0, true), clip(100, true)], "both")))
    await act(() => result.current.clearPicks(result.current.preview.data!.clips))
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(commands.packOverride).toHaveBeenCalledWith("p", "c", picks([[0], null]), DEFAULT_SPEC)
    expect(result.current.preview.data?.clips[0]).toMatchObject({ included: true, manual: null })
    expect(result.current.preview.data?.stamp).toBe("both")
  })

  it("hands picks back while the pack of new filters is on its way, then shows it as the backend has it", async () => {
    vi.mocked(commands.packGet).mockResolvedValue(state)
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, false, false), clip(100, true)], "one"))
    const { result } = renderHook(() => usePack("p", "c", new Map()), { wrapper })
    await waitFor(() => expect(result.current.current).toBe(true))
    // New filters: their pack's first fetch reads it before the hand-back, and comes late.
    const solo = { ...DEFAULT_SPEC, solo: true }
    let late: (p: PackPreview) => void = () => {}
    vi.mocked(commands.packPreview).mockImplementationOnce(() => new Promise((ok) => (late = ok)))
    act(() => result.current.setSpec(solo))
    await waitFor(() => expect(commands.packPreview).toHaveBeenCalledWith("p", "c", solo))
    expect(result.current.ready).toBe(false)
    vi.mocked(commands.packOverride).mockResolvedValue(choicesOf(preview([clip(0, true), clip(100, true)], "both")))
    vi.mocked(commands.packPreview).mockResolvedValue(preview([clip(0, true), clip(100, true)], "both, solo"))
    await act(async () => {
      await result.current.clearPicks(result.current.preview.data!.clips)
      late(preview([clip(0, false, false), clip(100, true)], "before the hand-back"))
    })
    await waitFor(() => expect(result.current.current).toBe(true))
    expect(commands.packOverride).toHaveBeenCalledWith("p", "c", picks([[0], null]), solo)
    expect(result.current.preview.data?.stamp).toBe("both, solo")
    expect(result.current.preview.data?.clips[0]).toMatchObject({ included: true, manual: null })
  })
})

describe("the pack's export", () => {
  const job = (id: string, exportId: string, state: JobInfo["state"], rev = 1) =>
    ({ id, kind: "export", projectId: "p", exportId, state, progress: null, rev, createdAt: rev }) as JobInfo

  it("is the job of the export the pack started, while it waits or runs", () => {
    useActivity.setState({ jobs: {}, order: [] })
    const { result } = renderHook(() => usePackExport("p", "walter"))
    expect(result.current).toBeUndefined()
    act(() => {
      packExportStarted("p", "walter", "e1")
      // Another character's pack, and Walter's in another project.
      packExportStarted("p", "jesse", "e2")
      packExportStarted("q", "walter", "e3")
      for (const j of [job("j2", "e2", "running"), job("j3", "e3", "running"), job("j1", "e1", "queued")]) {
        useActivity.getState().upsert(j)
      }
    })
    expect(result.current?.id).toBe("j1")
    act(() => useActivity.getState().upsert(job("j1", "e1", "cancelled", 2)))
    expect(result.current).toBeUndefined()
  })
})
