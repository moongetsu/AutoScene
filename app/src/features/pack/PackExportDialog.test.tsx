import type * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { Character, ExportPreset, PackNaming } from "@/bindings"
import { commands } from "@/lib/ipc"
import { queryClient } from "@/lib/queries"
import { PackExportDialog } from "./PackExportDialog"
import { DEFAULT_SPEC } from "./spec"
import type { Pack } from "./use-pack"

const prores: ExportPreset = {
  id: "prores-hq",
  name: "ProRes 422 HQ",
  description: "Edit-grade intra codec.",
  container: "mov",
  video: { codec: "prores", profile: "hq" },
  audio: { codec: "pcm24" },
  maxHeight: null,
  hdr: "auto",
  stereo: false,
  builtin: true,
}

vi.mock("@/lib/ipc", () => ({
  commands: {
    exportPresets: vi.fn(() => Promise.resolve([prores])),
    settingsGet: vi.fn(() =>
      Promise.resolve({ libraryRoot: "C:\\Library", exportPresetId: "prores-hq", audioLanguage: "eng" }),
    ),
    settingsUpdate: vi.fn(),
    exportDefaultDir: vi.fn(() => Promise.resolve("C:\\Library\\Exports\\Show")),
    packExport: vi.fn(() => Promise.resolve({})),
  },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("@/lib/media", () => ({ pickFolder: vi.fn() }))

const walter = { id: "c", label: "Walter" } as Character

/** A pack of three shots, as `usePack` hands it over. */
function pack(naming: PackNaming): Pack {
  return {
    state: { spec: DEFAULT_SPEC, naming },
    preview: { data: { clips: [], totals: { clips: 3, frames: 72, seconds: 3 }, stamp: "s" } },
    plan: {
      data: {
        names: ["Show_Walter_E01_00-00-00-00", "Show_Walter_E01_00-00-02-02", "Show_Walter_E01_00-00-12-02"],
        sizes: [{ presetId: "prores-hq", bytes: 3e8 }],
        folder: naming.folder && !naming.merge ? "Walter" : null,
        file: naming.merge ? "Show_Walter_scenepack" : null,
      },
      isError: false,
      error: null,
    },
    setNaming: vi.fn(),
    current: true,
  } as unknown as Pack
}

const naming = (merge: boolean): PackNaming => ({
  template: "{series} {character} {episode} {tc}",
  folder: true,
  merge,
  file: "{series} {character} scenepack",
})

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
)

afterEach(() => {
  cleanup()
  queryClient.clear()
  vi.clearAllMocks()
})

describe("PackExportDialog", () => {
  it("exports the pack into one file, with the shots' names as chapters", async () => {
    const p = pack(naming(true))
    render(<PackExportDialog pid="p" character={walter} pack={p} open onOpenChange={() => {}} />, { wrapper })
    await waitFor(() => expect(screen.getByText("Show_Walter_scenepack.mov")).toBeTruthy())
    expect(screen.getByLabelText("File name")).toBeTruthy()
    expect(screen.getByLabelText("Chapter names")).toBeTruthy()
    // One file has no folder of its own.
    expect(screen.queryByText(/In a folder named after/)).toBeNull()
    expect(screen.getByText("C:\\Library\\Exports\\Show")).toBeTruthy()
    // Making it takes room for the shots while they are joined.
    expect(screen.getByText(/needs extra free space/)).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "Export 3 shots into one file" }))
    await waitFor(() => expect(commands.packExport).toHaveBeenCalled())
    expect(vi.mocked(commands.packExport).mock.calls[0][2]).toMatchObject({ naming: { merge: true }, presetId: "prores-hq" })
  })

  it("switches to a file per shot", async () => {
    const p = pack(naming(true))
    render(<PackExportDialog pid="p" character={walter} pack={p} open onOpenChange={() => {}} />, { wrapper })
    fireEvent.click(await screen.findByRole("switch"))
    expect(p.setNaming).toHaveBeenCalledWith({ ...naming(true), merge: false })
  })

  it("names a file per shot, in the character's folder", async () => {
    const p = pack(naming(false))
    render(<PackExportDialog pid="p" character={walter} pack={p} open onOpenChange={() => {}} />, { wrapper })
    await waitFor(() => expect(screen.getByText("C:\\Library\\Exports\\Show\\Walter")).toBeTruthy())
    expect(screen.queryByLabelText("File name")).toBeNull()
    expect(screen.getByLabelText("File names")).toBeTruthy()
    expect(screen.getByText(/In a folder named after Walter/)).toBeTruthy()
    expect(screen.getByRole("button", { name: "Export 3 shots" })).toBeTruthy()
  })
})
