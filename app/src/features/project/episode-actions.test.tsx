import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { JobInfo, MediaItem } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { commands } from "@/lib/ipc"
import { useActivity } from "@/state/activity"
import { EpisodeItems } from "./episode-actions"

vi.mock("@/lib/ipc", () => ({
  commands: {
    mediaRetry: vi.fn(),
    mediaAnalyze: vi.fn(() => Promise.resolve(null)),
    jobCancel: vi.fn(() => Promise.resolve(null)),
  },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { error: vi.fn() }) }))

/** The items as plain buttons. */
const kit: MenuKit = {
  Item: ({ onSelect, children }: { onSelect?: () => void; children: React.ReactNode }) => (
    <button onClick={onSelect}>{children}</button>
  ),
  Separator: () => null,
}

/** An unfinished job of the episode "m". */
const job = (o: Partial<JobInfo>): JobInfo => ({
  id: "j",
  kind: "analyze",
  title: "Analysing E01.mkv",
  subtitle: null,
  projectId: "p",
  mediaId: "m",
  exportId: null,
  state: "running",
  progress: 0.4,
  stage: "Finding shots",
  etaMs: null,
  error: null,
  createdAt: 1,
  finishedAt: null,
  rev: 1,
  ...o,
})

const analysed = { id: "m", path: "C:\\Show\\E01.mkv", status: "ready", shots: "ready" } as MediaItem

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  useActivity.setState({ jobs: {}, order: [] })
})

describe("an episode's actions", () => {
  it("says so when indexing a failed episode again can't start", async () => {
    vi.mocked(commands.mediaRetry).mockRejectedValue("database is locked")
    const m = { id: "m", path: "C:\\Show\\E01.mkv", status: "failed", shots: "pending" } as MediaItem
    render(<EpisodeItems kit={kit} projectId="p" media={m} onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Index again/ }))
    expect(commands.mediaRetry).toHaveBeenCalledWith("p", "m")
    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Couldn't index it again", { description: "database is locked" }),
    )
  })

  it("cancels the analysis it names, not the preview built beside it", () => {
    // The preview's job is the newer one: what is cancelled is picked by kind.
    useActivity.getState().replaceAll([
      job({ id: "analysis", kind: "analyze", createdAt: 1 }),
      job({ id: "preview", kind: "proxy", stage: "Preparing preview", createdAt: 2, rev: 2 }),
    ])
    render(<EpisodeItems kit={kit} projectId="p" media={analysed} onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Cancel analysis/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("analysis")
    fireEvent.click(screen.getByRole("button", { name: /Cancel preview/ }))
    expect(commands.jobCancel).toHaveBeenLastCalledWith("preview")
  })

  it("keeps redoing and removing while it works", () => {
    // The backend restarts a running analysis once it stops, and stops the
    // work of what it removes: none of them waits for the job.
    useActivity.getState().replaceAll([job({ id: "analysis" })])
    const onRemove = vi.fn()
    render(<EpisodeItems kit={kit} projectId="p" media={analysed} onOpen={() => {}} onRemove={onRemove} />)
    fireEvent.click(screen.getByRole("button", { name: /Find shots again/ }))
    expect(commands.mediaAnalyze).toHaveBeenCalledWith("p", "m", "shots")
    fireEvent.click(screen.getByRole("button", { name: /Find characters again/ }))
    expect(commands.mediaAnalyze).toHaveBeenLastCalledWith("p", "m", "faces")
    fireEvent.click(screen.getByRole("button", { name: /Remove from project/ }))
    expect(onRemove).toHaveBeenCalled()
  })

  it("cancels a queued indexing as indexing, and offers nothing once its jobs are done", () => {
    useActivity.getState().replaceAll([job({ id: "ingest", kind: "ingest", state: "queued", progress: null })])
    const queued = { id: "m", path: "C:\\Show\\E01.mkv", status: "queued", shots: "pending" } as MediaItem
    const { unmount } = render(<EpisodeItems kit={kit} projectId="p" media={queued} onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Cancel indexing/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("ingest")
    expect(screen.queryByRole("button", { name: /Cancel analysis/ })).toBeNull()
    unmount()
    useActivity.getState().upsert(job({ id: "ingest", kind: "ingest", state: "cancelled", rev: 2 }))
    render(<EpisodeItems kit={kit} projectId="p" media={queued} onOpen={() => {}} onRemove={() => {}} />)
    expect(screen.queryByRole("button", { name: /Cancel/ })).toBeNull()
  })

  it("says so when a cancel can't be sent", async () => {
    vi.mocked(commands.jobCancel).mockRejectedValueOnce("the app is closing")
    useActivity.getState().replaceAll([job({ id: "analysis" })])
    render(<EpisodeItems kit={kit} projectId="p" media={analysed} onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Cancel analysis/ }))
    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Couldn't cancel the job", { description: "the app is closing" }),
    )
  })
})
