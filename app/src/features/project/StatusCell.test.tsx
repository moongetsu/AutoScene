import { QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { MediaItem, StageState } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { StatusCell } from "./StatusCell"

vi.mock("@/lib/ipc", () => ({
  commands: {
    settingsGet: vi.fn(() => Promise.resolve({ findBodies: true })),
    projectGet: vi.fn(() => Promise.resolve({ id: "p", fastScan: false })),
    mediaAnalyze: vi.fn(() => Promise.resolve(null)),
    mediaRetry: vi.fn(() => Promise.resolve(null)),
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
    render(<StatusCell media={episode("failed")} projectId="p" faces />, { wrapper })
    expect(await screen.findByText(/People failed/)).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }))
    expect(commands.mediaAnalyze).toHaveBeenCalledWith("p", "m", null)
  })

  it("goes on with people a stop or a quit cut short", async () => {
    const { unmount } = render(<StatusCell media={episode("stopped")} projectId="p" faces />, { wrapper })
    expect(await screen.findByRole("button", { name: /People stopped · continue/ })).toBeTruthy()
    unmount()
    render(<StatusCell media={episode("running")} projectId="p" faces />, { wrapper })
    fireEvent.click(await screen.findByRole("button", { name: /Find people/ }))
    expect(commands.mediaAnalyze).toHaveBeenCalledWith("p", "m", null)
  })

  it("says nothing of people when they aren't looked for", async () => {
    vi.mocked(commands.settingsGet).mockResolvedValueOnce({ findBodies: false } as never)
    render(<StatusCell media={episode("failed")} projectId="p" faces />, { wrapper })
    await waitFor(() => expect(queryClient.getQueryData(qk.settings)).toBeTruthy())
    await waitFor(() => expect(queryClient.getQueryData(qk.project("p"))).toBeTruthy())
    expect(screen.getByText(/faces/)).toBeTruthy()
    expect(screen.queryByText(/People/)).toBeNull()
  })

  it("says nothing of people with Fast Scan, which finds faces alone", async () => {
    vi.mocked(commands.projectGet).mockResolvedValueOnce({ id: "p", fastScan: true } as never)
    render(<StatusCell media={episode("pending")} projectId="p" faces />, { wrapper })
    await waitFor(() => expect(queryClient.getQueryData(qk.settings)).toBeTruthy())
    await waitFor(() => expect(queryClient.getQueryData(qk.project("p"))).toBeTruthy())
    expect(screen.queryByText(/people/i)).toBeNull()
  })
})
