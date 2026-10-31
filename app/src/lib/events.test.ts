import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { toast } from "sonner"
import type { ExportItem, ExportSummary, JobInfo } from "@/bindings"
import { useActivity } from "@/state/activity"
import { startEventBridge } from "./events"
import { commands } from "./ipc"
import { queryClient } from "./queries"

// Each backend event's listener, as the bridge registers it.
const { handlers } = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }))

vi.mock("./ipc", () => ({
  events: new Proxy(
    {},
    {
      get: (_, name: string) => ({
        listen: (h: (e: { payload: unknown }) => void) => {
          handlers[name] = h
          return Promise.resolve(() => {})
        },
      }),
    },
  ),
  commands: {
    jobsList: vi.fn(() => Promise.resolve([])),
    exportsList: vi.fn(),
    revealPath: vi.fn(() => Promise.resolve(null)),
  },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }))

const job = (o: Partial<JobInfo>): JobInfo => ({
  id: "j1",
  kind: "export",
  title: "Exporting 2 clips",
  subtitle: "Breaking Bad",
  projectId: "p",
  mediaId: null,
  exportId: "e1",
  state: "running",
  progress: null,
  stage: null,
  etaMs: null,
  error: null,
  createdAt: 1,
  finishedAt: null,
  rev: 1,
  ...o,
})

const clip = (path: string) => ({ id: 1, mediaId: "m", start: 0, end: 24, name: "clip", status: "done", path, verified: true }) as ExportItem
const exported = (o: Partial<ExportSummary>) =>
  ({ id: "e1", outDir: "C:\\Exports\\Show", items: [clip("a.mov"), clip("b.mov")], merged: null, ...o }) as ExportSummary

/** The job finishing, and the toast that says so. */
async function finish(list: ExportSummary[]) {
  vi.mocked(commands.exportsList).mockResolvedValue(list)
  handlers.jobChanged({ payload: job({ state: "done", rev: 2 }) })
  await vi.waitFor(() => expect(toast.success).toHaveBeenCalledOnce())
  return vi.mocked(toast.success).mock.calls[0][1] as { action?: { label: string; onClick: () => void } }
}

let stop: () => void
beforeEach(() => {
  useActivity.setState({ jobs: {}, order: [] })
  stop = startEventBridge()
  handlers.jobChanged({ payload: job({}) })
})
afterEach(() => {
  stop()
  queryClient.clear()
  vi.clearAllMocks()
})

describe("an export finishing", () => {
  it("offers to show the folder of a file per clip", async () => {
    const { action } = await finish([exported({})])
    expect(action?.label).toBe("Show in folder")
    action?.onClick()
    expect(commands.revealPath).toHaveBeenCalledWith("C:\\Exports\\Show")
  })

  it("shows the one file the clips were joined into", async () => {
    const merged = { name: "Pack", path: "C:\\Exports\\Show\\Pack.mov", verified: true, bytes: 1, error: null }
    const { action } = await finish([exported({ merged })])
    action?.onClick()
    expect(commands.revealPath).toHaveBeenCalledWith("C:\\Exports\\Show\\Pack.mov")
  })

  it("shows a single clip itself", async () => {
    const { action } = await finish([exported({ items: [clip("C:\\Exports\\Show\\clip.mov")] })])
    action?.onClick()
    expect(commands.revealPath).toHaveBeenCalledWith("C:\\Exports\\Show\\clip.mov")
  })

  it("says it finished without the button when the export can't be found", async () => {
    const { action } = await finish([])
    expect(action).toBeUndefined()
  })
})
