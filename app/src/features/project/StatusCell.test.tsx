import { QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { JobInfo, MediaItem, StageState } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { useActivity } from "@/state/activity"
import { StatusCell } from "./StatusCell"

vi.mock("@/lib/ipc", () => ({
  commands: {
    settingsGet: vi.fn(() => Promise.resolve({ findBodies: true })),
    projectGet: vi.fn(() => Promise.resolve({ id: "p", fastScan: false })),
    mediaAnalyze: vi.fn(() => Promise.resolve(null)),
    mediaRetry: vi.fn(() => Promise.resolve(null)),
    jobCancel: vi.fn(() => Promise.resolve(null)),
  },
  errorMessage: (e: unknown) => String(e),
}))

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider>{children}</TooltipProvider>
  </QueryClientProvider>
)

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.clearAllMocks()
  useActivity.setState({ jobs: {}, order: [] })
})

/** A job of the episode "m". */
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

/** An episode whose shots and faces are found, its people `bodies`. */
function episode(bodies: StageState): MediaItem {
  return {
    id: "m",
    status: "ready",
    shots: "ready",
    shotCount: 12,
    faces: "ready",
    faceCount: 30,
    bodies,
    bodiesError: bodies === "failed" ? "The engine stopped." : null,
  } as MediaItem
}

describe("StatusCell", () => {
  it("offers to find the people again when their pass failed", async () => {
    render(<StatusCell media={episode("failed")} projectId="p" />, { wrapper })
    expect(await screen.findByText(/People failed/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }))
    expect(commands.mediaAnalyze).toHaveBeenCalledWith("p", "m", null)
  })

  it("goes on with people a stop or a quit cut short", async () => {
    const { unmount } = render(<StatusCell media={episode("stopped")} projectId="p" />, { wrapper })
    expect(await screen.findByRole("button", { name: /People stopped · continue/ })).toBeTruthy()
    unmount()
    render(<StatusCell media={episode("running")} projectId="p" />, { wrapper })
    fireEvent.click(await screen.findByRole("button", { name: /Find people/ }))
    expect(commands.mediaAnalyze).toHaveBeenCalledWith("p", "m", null)
  })

  it("says nothing of people when they aren't looked for", async () => {
    vi.mocked(commands.settingsGet).mockResolvedValueOnce({ findBodies: false } as never)
    render(<StatusCell media={episode("failed")} projectId="p" />, { wrapper })
    await waitFor(() => expect(queryClient.getQueryData(qk.settings)).toBeTruthy())
    await waitFor(() => expect(queryClient.getQueryData(qk.project("p"))).toBeTruthy())
    expect(screen.getByText(/faces/)).toBeTruthy()
    expect(screen.queryByText(/People/)).toBeNull()
  })

  it("says nothing of people in anime, whose characters are found whole", async () => {
    vi.mocked(commands.projectGet).mockResolvedValueOnce({ id: "p", fastScan: false, content: "anime" } as never)
    render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    await waitFor(() => expect(queryClient.getQueryData(qk.settings)).toBeTruthy())
    await waitFor(() => expect(queryClient.getQueryData(qk.project("p"))).toBeTruthy())
    expect(screen.getByText(/faces/)).toBeTruthy()
    expect(screen.queryByText(/people/i)).toBeNull()
  })

  it("says nothing of people with Fast Scan, which finds faces alone", async () => {
    vi.mocked(commands.projectGet).mockResolvedValueOnce({ id: "p", fastScan: true } as never)
    render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    await waitFor(() => expect(queryClient.getQueryData(qk.settings)).toBeTruthy())
    await waitFor(() => expect(queryClient.getQueryData(qk.project("p"))).toBeTruthy())
    expect(screen.queryByText(/people/i)).toBeNull()
  })

  it("cancels the job whose progress it shows, named by its kind", async () => {
    useActivity.getState().replaceAll([job({ id: "analysis" })])
    const { unmount } = render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    fireEvent.click(await screen.findByRole("button", { name: "Cancel analysis" }))
    expect(commands.jobCancel).toHaveBeenCalledWith("analysis")
    unmount()
    // A preview being built is never cancelled as "analysis".
    useActivity.getState().replaceAll([job({ id: "preview", kind: "proxy", stage: null, createdAt: 2, rev: 2 })])
    render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    expect(screen.getByText("Preparing preview")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Cancel preview" }))
    expect(commands.jobCancel).toHaveBeenLastCalledWith("preview")
    expect(screen.queryByRole("button", { name: "Cancel analysis" })).toBeNull()
  })

  it("tells the time left beside the progress, once there is an estimate", async () => {
    useActivity.getState().replaceAll([job({ etaMs: 150_000 })])
    const { unmount } = render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    expect(await screen.findByText("about 3 min left")).toBeTruthy()
    expect(screen.getByText("40%")).toBeTruthy()
    unmount()
    useActivity.getState().replaceAll([job({ etaMs: null, rev: 2 })])
    render(<StatusCell media={episode("pending")} projectId="p" />, { wrapper })
    expect(screen.getByText("40%")).toBeTruthy()
    expect(screen.queryByText(/left/)).toBeNull()
  })

  it("cancels an indexing that waits its turn", async () => {
    useActivity.getState().replaceAll([job({ id: "ingest", kind: "ingest", state: "queued", progress: null })])
    render(<StatusCell media={{ id: "m", status: "queued", shots: "pending" } as MediaItem} projectId="p" />, { wrapper })
    expect(screen.getByText("Queued")).toBeTruthy()
    fireEvent.click(await screen.findByRole("button", { name: "Cancel indexing" }))
    expect(commands.jobCancel).toHaveBeenCalledWith("ingest")
  })
})
