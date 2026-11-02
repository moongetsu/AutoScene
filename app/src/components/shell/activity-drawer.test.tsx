import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { JobInfo } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useActivity } from "@/state/activity"
import { useUi } from "@/state/ui"
import { ActivityDrawer } from "./ActivityDrawer"
import { AppMenu } from "./AppMenu"

const { copyText } = vi.hoisted(() => ({ copyText: vi.fn() }))

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => vi.fn(),
  useCanGoBack: () => false,
}))
vi.mock("@/lib/os", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/lib/os")>()), copyText }))

const job = (o: Partial<JobInfo>) =>
  ({
    id: "j1",
    kind: "plugin",
    title: "Post to Discord",
    subtitle: "Discord notify",
    projectId: "p",
    mediaId: null,
    exportId: "e1",
    state: "running",
    progress: null,
    stage: null,
    etaMs: null,
    error: null,
    message: null,
    createdAt: 1,
    finishedAt: null,
    rev: 1,
    ...o,
  }) as JobInfo

function show(...jobs: JobInfo[]) {
  useActivity.setState({ jobs: Object.fromEntries(jobs.map((j) => [j.id, j])), order: jobs.map((j) => j.id) })
  render(
    <AppMenu>
      <div>
        <ActivityDrawer />
      </div>
    </AppMenu>,
    { wrapper: TooltipProvider },
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  useUi.setState({ activityOpen: true })
})
afterEach(() => {
  cleanup()
  useUi.setState({ activityOpen: false })
  useActivity.setState({ jobs: {}, order: [] })
})

describe("the Activity drawer", () => {
  it("stops a plugin's run, and cancels other work", () => {
    show(job({}), job({ id: "j2", kind: "ingest", title: "Indexing Pilot", exportId: null }))
    expect(screen.getByRole("button", { name: "Stop Post to Discord" })).toBeTruthy()
    expect(screen.getByRole("button", { name: "Cancel Indexing Pilot" })).toBeTruthy()
  })

  it("shows what a finished plugin had to say, and copies it", async () => {
    show(job({ state: "done", message: "Posted 12 clips.", finishedAt: 2 }))
    expect(screen.getByText("Posted 12 clips.")).toBeTruthy()
    fireEvent.contextMenu(screen.getByText("Post to Discord"), { clientX: 40, clientY: 40 })
    fireEvent.click(await screen.findByRole("menuitem", { name: "Copy the message" }))
    await waitFor(() => expect(copyText).toHaveBeenCalledExactlyOnceWith("Posted 12 clips.", "message"))
  })

  it("shows no message for a job that failed", () => {
    show(job({ state: "failed", error: "No webhook is set.", message: "Posted 12 clips." }))
    expect(screen.getByText("No webhook is set.")).toBeTruthy()
    expect(screen.queryByText("Posted 12 clips.")).toBeNull()
  })
})
