import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { ExportItem, ExportStatus, ExportSummary, JobInfo, MediaItem } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useActivity } from "@/state/activity"
import { ExportsPage } from "./ExportsPage"

const { list } = vi.hoisted(() => ({ list: { exports: [] as ExportSummary[], media: [] as MediaItem[] } }))

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ pid: "p" }),
  useNavigate: () => vi.fn(),
}))
// The page around it scrolls and collects menus; here only the cards matter.
vi.mock("@/components/ui/page", () => ({ Page: ({ children }: { children: React.ReactNode }) => <main>{children}</main> }))
vi.mock("@/lib/queries", () => ({
  useProject: () => ({ data: { title: "Show" } }),
  useExports: () => ({ data: list.exports }),
  useMediaList: () => ({ data: list.media }),
  firstFailure: () => null,
}))

const item = (id: number, status: ExportStatus, error: string | null = null) =>
  ({ id, mediaId: "m", start: id * 100, end: id * 100 + 24, name: `Clip_${id}`, status, path: null, verified: status === "done", error }) as ExportItem

const exported = (status: ExportStatus, items: ExportItem[]): ExportSummary =>
  ({ id: "e1", projectId: "p", createdAt: 1, preset: { name: "ProRes 422 HQ", container: "mov" }, outDir: "C:\\Out", status, items, label: null, merged: null }) as ExportSummary

const job = (state: JobInfo["state"], progress: number | null = null) =>
  ({ id: "j1", kind: "export", title: "Exporting", projectId: "p", exportId: "e1", state, progress, rev: 1, createdAt: 1 }) as JobInfo

function show(ex: ExportSummary, j?: JobInfo, media: MediaItem[] = []) {
  list.exports = [ex]
  list.media = media
  useActivity.setState(j ? { jobs: { [j.id]: j }, order: [j.id] } : { jobs: {}, order: [] })
  render(<ExportsPage />, { wrapper: TooltipProvider })
}

beforeEach(() => useActivity.setState({ jobs: {}, order: [] }))
afterEach(cleanup)

describe("an export's card", () => {
  it("shows a running export's progress", () => {
    show(exported("running", [item(1, "running"), item(2, "queued")]), job("running", 0.4))
    expect(screen.getByText("Exporting")).toBeTruthy()
    expect(screen.getByRole("progressbar")).toBeTruthy()
    expect(screen.getByText("40%")).toBeTruthy()
  })

  it("says Cancelled as soon as its job is, before the export is read again", () => {
    show(exported("running", [item(1, "done"), item(2, "running")]), job("cancelled", 0.4))
    expect(screen.getByText("Cancelled")).toBeTruthy()
    expect(screen.queryByText("Exporting")).toBeNull()
    expect(screen.queryByRole("progressbar")).toBeNull()
  })

  it("no longer says Queued once its job was cancelled", () => {
    show(exported("queued", [item(1, "queued")]), job("cancelled"))
    expect(screen.getByText("Cancelled")).toBeTruthy()
    expect(screen.queryByText("Queued")).toBeNull()
  })
})

describe("an export's clip list", () => {
  const six = (status: ExportStatus) => Array.from({ length: 6 }, (_, i) => item(i + 1, status))
  const toggle = () => screen.getByRole("button", { name: /clips/ })

  it("starts folded once the export is done, and opens", () => {
    show(exported("done", six("done")), undefined, [{ id: "m", label: "E01", fps: { num: 24, den: 1 } } as MediaItem])
    // Six clips of a second each.
    expect(toggle().textContent).toBe("Show 6 clips · 0:06")
    expect(toggle().getAttribute("aria-expanded")).toBe("false")
    expect(screen.queryByText("Clip_1")).toBeNull()
    fireEvent.click(toggle())
    expect(toggle().textContent).toBe("Hide clips")
    expect(screen.getAllByText(/^Clip_/)).toHaveLength(6)
  })

  it("keeps the clips that failed in sight while folded", () => {
    const items = six("done")
    items[2] = item(3, "failed", "The source file is gone.")
    show(exported("failed", items))
    expect(toggle().textContent).toBe("Show all 6 clips")
    expect(screen.getAllByText(/^Clip_/).map((e) => e.textContent)).toEqual(["Clip_3"])
    expect(screen.getByText("The source file is gone.")).toBeTruthy()
  })

  it("is open while the export runs", () => {
    show(exported("running", six("running")), job("running", 0.2))
    expect(toggle().textContent).toBe("Hide clips")
    expect(screen.getAllByText(/^Clip_/)).toHaveLength(6)
  })

  it("isn't folded with only a few clips", () => {
    show(exported("done", six("done").slice(0, 4)))
    expect(screen.queryByRole("button", { name: /clips/ })).toBeNull()
    expect(screen.getAllByText(/^Clip_/)).toHaveLength(4)
  })
})
