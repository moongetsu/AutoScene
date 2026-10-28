import { afterEach, describe, expect, it, vi } from "vitest"
import type { AppSettings } from "@/bindings"
import { commands } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { rememberPreset } from "./export"

vi.mock("@/lib/ipc", () => ({
  commands: { settingsUpdate: vi.fn() },
  errorMessage: (e: unknown) => String(e),
}))

const settings = { libraryRoot: "C:\\Library", exportPresetId: "prores-hq", audioLanguage: "eng" } as AppSettings

afterEach(() => {
  queryClient.clear()
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

describe("remembering the export format", () => {
  it("saves a format other than the remembered one", async () => {
    vi.mocked(commands.settingsUpdate).mockImplementation((s) => Promise.resolve(s))
    rememberPreset(settings, "prores-hq")
    rememberPreset(undefined, "h264")
    expect(commands.settingsUpdate).not.toHaveBeenCalled()
    rememberPreset(settings, "h264")
    expect(commands.settingsUpdate).toHaveBeenCalledWith({ ...settings, exportPresetId: "h264" })
    await vi.waitFor(() => expect(queryClient.getQueryData(qk.settings)).toMatchObject({ exportPresetId: "h264" }))
  })

  it("notes a failure in the log instead of leaving it unhandled", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {})
    vi.mocked(commands.settingsUpdate).mockRejectedValue("library drive unplugged")
    rememberPreset(settings, "h264")
    await vi.waitFor(() => expect(warn).toHaveBeenCalledWith("remembering the export format:", "library drive unplugged"))
    expect(queryClient.getQueryData(qk.settings)).toBeUndefined()
  })
})
