import type * as React from "react"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { ExportItem, ExportStatus, ExportSummary, JobInfo, MediaItem, PluginInfo } from "@/bindings"
import { AppMenu } from "@/components/shell/AppMenu"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useActivity } from "@/state/activity"
import { pluginInfo } from "@/test/plugins"
import { ExportsPage } from "./ExportsPage"

const { list, pickSaveFile, exportSaveList, pluginRun, reveal } = vi.hoisted(() => ({
  list: { exports: [] as ExportSummary[], media: [] as MediaItem[], plugins: [] as PluginInfo[] },
  pickSaveFile: vi.fn(),
  exportSaveList: vi.fn(),
  pluginRun: vi.fn(),
  reveal: vi.fn(),
}))

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ pid: "p" }),
  useNavigate: () => vi.fn(),
  useCanGoBack: () => false,
}))
vi.mock("@/lib/media", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/lib/media")>()), pickSaveFile }))
vi.mock("@/lib/ipc", async (importOriginal) => {
  const ipc = await importOriginal<typeof import("@/lib/ipc")>()
  return { ...ipc, commands: { ...ipc.commands, exportSaveList, pluginRun } }
})
vi.mock("@/lib/os", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/lib/os")>()), reveal }))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }))
// The page around it scrolls and collects menus; here only the cards matter.
vi.mock("@/components/ui/page", () => ({ Page: ({ children }: { children: React.ReactNode }) => <main>{children}</main> }))
vi.mock("@/lib/queries", () => ({
  useProject: () => ({ data: { title: "Show" } }),
  useExports: () => ({ data: list.exports }),
  useMediaList: () => ({ data: list.media }),
  usePlugins: () => ({ data: list.plugins }),
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
  render(
    <AppMenu>
      <div>
        <ExportsPage />
      </div>
    </AppMenu>,
    { wrapper: TooltipProvider },
  )
}

/** The export card's menu, opened from its header. */
async function openMenu() {
  fireEvent.contextMenu(screen.getByRole("heading", { level: 2 }), { clientX: 40, clientY: 40 })
  await screen.findByRole("menu")
}

beforeEach(() => {
  vi.clearAllMocks()
  list.plugins = []
  useActivity.setState({ jobs: {}, order: [] })
})
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

  it("goes by its own job, never a plugin's run on that export", () => {
    const run = (state: JobInfo["state"], progress: number | null = null) =>
      ({ ...job(state, progress), id: "j2", kind: "plugin", title: "Post to Discord" }) as JobInfo
    show(exported("running", [item(1, "done"), item(2, "running")]), run("running", 0.7))
    expect(screen.getByText("Exporting")).toBeTruthy()
    expect(screen.queryByText("70%")).toBeNull()
    cleanup()

    show(exported("running", [item(1, "done"), item(2, "running")]), run("cancelled"))
    expect(screen.getByText("Exporting")).toBeTruthy()
    expect(screen.queryByText("Cancelled")).toBeNull()
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

describe("an export's edit lists", () => {
  const entry = (name: string) => screen.getByRole("menuitem", { name })
  const merged = (path: string | null): ExportSummary => ({
    ...exported("running", [item(1, "done")]),
    merged: { name: "Walter", path, verified: false, bytes: null, error: null },
  })

  it("offers Save EDL… once a clip is done", async () => {
    show(exported("running", [item(1, "running"), item(2, "queued")]), job("running", 0.1))
    await openMenu()
    expect(entry("Save EDL…").getAttribute("aria-disabled")).toBe("true")
    expect(entry("Save cut list (CSV)…").getAttribute("aria-disabled")).toBe("true")
    cleanup()

    show(exported("running", [item(1, "done"), item(2, "running")]), job("running", 0.6))
    await openMenu()
    expect(entry("Save EDL…").getAttribute("aria-disabled")).toBeNull()
    expect(entry("Save cut list (CSV)…").getAttribute("aria-disabled")).toBeNull()
    cleanup()

    // One file: not before it's made, however many clips are done.
    show(merged(null), job("running", 0.9))
    await openMenu()
    expect(entry("Save EDL…").getAttribute("aria-disabled")).toBe("true")
    cleanup()

    show(merged(String.raw`C:\Out\Walter.mp4`), job("done", 1))
    await openMenu()
    expect(entry("Save EDL…").getAttribute("aria-disabled")).toBeNull()
    expect(entry("Save cut list (CSV)…").getAttribute("aria-disabled")).toBeNull()
  })

  it("lists the same in the card's ⋯ menu", async () => {
    show(exported("done", [item(1, "done")]))
    fireEvent.pointerDown(screen.getByRole("button", { name: "ProRes 422 HQ options" }), { button: 0, ctrlKey: false })
    const menu = await screen.findByRole("menu")
    expect(within(menu).getAllByRole("menuitem").map((i) => i.textContent?.trim())).toEqual([
      "Show folder",
      "Copy folder path",
      "Save EDL…",
      "Save cut list (CSV)…",
    ])
  })

  it("saves the EDL where the person chose", async () => {
    pickSaveFile.mockResolvedValue(String.raw`D:\Edit\Walter.edl`)
    exportSaveList.mockResolvedValue({ path: String.raw`D:\Edit\Walter.edl`, leftOut: 0 })
    show({ ...exported("done", [item(1, "done")]), label: "Walter · scenepack" })
    await openMenu()
    fireEvent.click(entry("Save EDL…"))
    // Offered beside the export's files, named after it.
    await waitFor(() => expect(exportSaveList).toHaveBeenCalledWith("p", "e1", "edl", String.raw`D:\Edit\Walter.edl`))
    expect(pickSaveFile).toHaveBeenCalledWith("Save EDL", String.raw`C:\Out\Walter · scenepack.edl`, { name: "EDL", extensions: ["edl"] })
    // Said, with the way to it.
    await waitFor(() => expect(toast).toHaveBeenCalledOnce())
    const [title, options] = vi.mocked(toast).mock.calls[0] as [string, { description?: string; action: { label: string; onClick: () => void } }]
    expect([title, options.description, options.action.label]).toEqual(["Saved the EDL", undefined, "Show in folder"])
    options.action.onClick()
    expect(reveal).toHaveBeenCalledWith(String.raw`D:\Edit\Walter.edl`)
  })

  it("saves a cut list as a CSV, and says which clips it left out", async () => {
    pickSaveFile.mockResolvedValue("/home/me/Walter.csv")
    exportSaveList.mockResolvedValue({ path: "/home/me/Walter.csv", leftOut: 2 })
    show({ ...exported("done", [item(1, "done")]), outDir: "/home/me/Out/" })
    await openMenu()
    fireEvent.click(entry("Save cut list (CSV)…"))
    await waitFor(() => expect(exportSaveList).toHaveBeenCalledWith("p", "e1", "csv", "/home/me/Walter.csv"))
    expect(pickSaveFile).toHaveBeenCalledWith("Save cut list", "/home/me/Out/ProRes 422 HQ.csv", { name: "CSV", extensions: ["csv"] })
    await waitFor(() =>
      expect(toast).toHaveBeenCalledWith("Saved the cut list", expect.objectContaining({ description: "2 clips were left out: their episodes were removed." })),
    )
  })

  it("says why a list couldn't be saved, and nothing when the dialog is closed", async () => {
    pickSaveFile.mockResolvedValue(null)
    show(exported("done", [item(1, "done")]))
    await openMenu()
    fireEvent.click(entry("Save EDL…"))
    await waitFor(() => expect(pickSaveFile).toHaveBeenCalledOnce())
    expect(exportSaveList).not.toHaveBeenCalled()
    cleanup()

    pickSaveFile.mockResolvedValue(String.raw`D:\Edit\Walter.edl`)
    exportSaveList.mockRejectedValue({ code: "invalid", message: "Clips of different frame rates can't share one EDL." })
    show(exported("done", [item(1, "done")]))
    await openMenu()
    fireEvent.click(entry("Save EDL…"))
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Couldn't save the EDL", { description: "Clips of different frame rates can't share one EDL." }),
    )
    expect(toast).not.toHaveBeenCalled()
  })
})

describe("an export's plugin actions", () => {
  const post = { id: "post", title: "Post to Discord", description: null, afterExport: false }
  const entry = (name: RegExp) => screen.getByRole("menuitem", { name })

  it("runs a plugin's export action", async () => {
    list.plugins = [
      pluginInfo({ id: "dev.example.discord-notify", name: "Discord Notify", exportActions: [post] }),
      // One that may not run now adds nothing.
      pluginInfo({ id: "dev.a.off", name: "Off", active: false, exportActions: [{ ...post, id: "off", title: "Never shown" }] }),
    ]
    pluginRun.mockResolvedValue("j9")
    show(exported("done", [item(1, "done")]))
    await openMenu()
    expect(screen.queryByRole("menuitem", { name: /Never shown/ })).toBeNull()
    // Named with its plugin.
    expect(entry(/Post to Discord/).textContent).toContain("Discord Notify")
    fireEvent.click(entry(/Post to Discord/))
    await waitFor(() => expect(pluginRun).toHaveBeenCalledExactlyOnceWith("dev.example.discord-notify", "post", "exportAction", "p", "e1"))
    expect(toast.error).not.toHaveBeenCalled()
    cleanup()

    // Not while the export still runs.
    show(exported("running", [item(1, "running")]), job("running", 0.3))
    await openMenu()
    expect(entry(/Post to Discord/).getAttribute("aria-disabled")).toBe("true")
  })

  it("says why a plugin's run couldn't start, or that it runs already", async () => {
    list.plugins = [pluginInfo({ id: "dev.example.discord-notify", name: "Discord Notify", exportActions: [post] })]
    pluginRun.mockResolvedValue(null)
    show(exported("done", [item(1, "done")]))
    await openMenu()
    fireEvent.click(entry(/Post to Discord/))
    await waitFor(() => expect(toast).toHaveBeenCalledWith("Post to Discord is already waiting or running", { description: "It's in Activity." }))
    cleanup()

    pluginRun.mockRejectedValue({ code: "invalid", message: "Discord Notify is turned off." })
    show(exported("done", [item(1, "done")]))
    await openMenu()
    fireEvent.click(entry(/Post to Discord/))
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("Couldn't run Post to Discord", { description: "Discord Notify is turned off." }))
  })
})
