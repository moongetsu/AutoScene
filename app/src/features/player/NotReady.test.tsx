import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { FrameCount, JobInfo, MediaItem } from "@/bindings"
import { commands } from "@/lib/ipc"
import { useActivity } from "@/state/activity"
import { NotReady } from "./NotReady"

vi.mock("@/lib/ipc", () => ({
  commands: {
    jobCancel: vi.fn(() => Promise.resolve(null)),
    mediaIndexingStill: vi.fn(() => Promise.resolve(null)),
  },
  errorMessage: (e: unknown) => String(e),
  isAppError: (e: unknown) => typeof e === "object" && e !== null && "code" in e,
}))

/** A running job of the episode "m". */
const job = (o: Partial<JobInfo>): JobInfo => ({
  id: "j",
  kind: "ingest",
  title: "Indexing E01.mkv",
  subtitle: null,
  projectId: "p",
  mediaId: "m",
  exportId: null,
  state: "running",
  progress: 0.4,
  stage: "Indexing frames",
  etaMs: null,
  error: null,
  createdAt: 1,
  finishedAt: null,
  rev: 1,
  ...o,
})

const episode = (o: Partial<MediaItem>) => ({ id: "m", path: "C:\\Show\\E01.mkv", poster: null, ...o }) as MediaItem

const show = (media: MediaItem, error: unknown = null) =>
  render(<NotReady pid="p" media={media} error={error} gone={false} listError={null} onRetryList={() => {}} />)

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  useActivity.setState({ jobs: {}, order: [] })
})

describe("the player while an episode isn't ready", () => {
  it("cancels the indexing it waits for, and says how long it has left", () => {
    useActivity.getState().replaceAll([job({ id: "ingest", etaMs: 150_000 })])
    show(episode({ status: "indexing" }))
    expect(screen.getByText("about 3 min left · 40%")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Cancel indexing/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("ingest")
    // Still reading the file: there is no place in it to show yet.
    expect(commands.mediaIndexingStill).not.toHaveBeenCalled()
  })

  it("counts the frames the indexing reads, and tells its pace and time left only once measured", async () => {
    const frames = (o: Partial<FrameCount>): FrameCount => ({ done: 14_200, total: 88_500, rate: 24, perSec: null, ...o })
    useActivity.getState().replaceAll([job({ id: "ingest", progress: 0.16, etaMs: 150_000, frames: frames({}) })])
    const { container } = show(episode({ status: "indexing" }))
    expect(container.textContent).toContain("14,200 of about 88,500 frames")
    expect(screen.getByText("16%")).toBeTruthy()
    expect(screen.getByText("Measuring speed…")).toBeTruthy()
    expect(screen.queryByText(/left/)).toBeNull()
    act(() =>
      useActivity.getState().progress({
        id: "ingest",
        rev: 2,
        progress: 0.17,
        stage: "Indexing frames",
        etaMs: 150_000,
        frames: frames({ done: 15_000, perSec: 600 }),
      }),
    )
    expect(container.textContent).toContain("600 fps · 25× real time")
    expect(screen.getByText("about 3 min left")).toBeTruthy()
    // Past what the file says it has, only what was read is told.
    act(() =>
      useActivity.getState().progress({
        id: "ingest",
        rev: 3,
        progress: 0.95,
        stage: "Indexing frames",
        etaMs: 1000,
        frames: frames({ done: 90_100, perSec: 600 }),
      }),
    )
    expect(container.textContent).toContain("90,100 frames")
    expect(container.textContent).not.toContain("of about")
    await waitFor(() => expect(commands.mediaIndexingStill).toHaveBeenCalledWith("p", "m"))
  })

  it("shows where the indexing is as a picture once one comes", async () => {
    const still = "data:image/jpeg;base64,AAAA"
    vi.mocked(commands.mediaIndexingStill).mockResolvedValueOnce(still)
    const frames = { done: 1200, total: 34_000, rate: 24, perSec: null }
    useActivity.getState().replaceAll([job({ id: "ingest", frames })])
    const { container } = show(episode({ status: "indexing" }))
    await waitFor(() => expect(container.querySelectorAll(`img[src="${still}"]`).length).toBe(2))
  })

  it("asks for no picture while the indexing waits its turn", () => {
    useActivity.getState().replaceAll([job({ id: "ingest", state: "queued", progress: null })])
    show(episode({ status: "queued" }))
    expect(screen.getByText("Waiting…")).toBeTruthy()
    expect(commands.mediaIndexingStill).not.toHaveBeenCalled()
  })

  it("shows the preview's progress, not an analysis running beside it", () => {
    useActivity.getState().replaceAll([
      job({ id: "preview", kind: "proxy", stage: "Preparing preview", progress: 0.2, createdAt: 1 }),
      job({ id: "analysis", kind: "analyze", stage: "Finding shots", progress: 0.7, createdAt: 2, rev: 2 }),
    ])
    show(episode({ status: "ready", proxy: "pending" }), { code: "not_ready", message: "The preview isn't ready yet." })
    expect(screen.getByText("Preparing preview", { selector: "span" })).toBeTruthy()
    expect(screen.getByText("20%")).toBeTruthy()
    expect(screen.queryByText("Finding shots")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: /Cancel preview/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("preview")
    expect(screen.queryByRole("button", { name: /Cancel analysis/ })).toBeNull()
  })
})
