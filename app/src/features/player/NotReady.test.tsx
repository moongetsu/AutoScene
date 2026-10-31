import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { JobInfo, MediaItem } from "@/bindings"
import { commands } from "@/lib/ipc"
import { useActivity } from "@/state/activity"
import { NotReady } from "./NotReady"

vi.mock("@/lib/ipc", () => ({
  commands: { jobCancel: vi.fn(() => Promise.resolve(null)) },
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
  it("cancels the indexing it waits for", () => {
    useActivity.getState().replaceAll([job({ id: "ingest" })])
    show(episode({ status: "indexing" }))
    fireEvent.click(screen.getByRole("button", { name: /Cancel indexing/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("ingest")
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
