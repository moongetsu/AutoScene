import { afterEach, describe, expect, it, vi } from "vitest"
import { commands } from "@/lib/ipc"
import { releaseNotesUrl, startUpdateSchedule, useUpdates } from "./updates"

vi.mock("@/lib/ipc", () => ({
  commands: { updateCheck: vi.fn() },
  errorMessage: (e: unknown) => String(e),
  events: {},
}))

afterEach(() => {
  vi.useRealTimers()
  vi.mocked(commands.updateCheck).mockReset()
  useUpdates.getState().setAuto(true)
})

describe("update checks", () => {
  it("look by themselves at launch unless that is turned off", async () => {
    vi.useFakeTimers()
    vi.mocked(commands.updateCheck).mockResolvedValue({ state: "none" } as never)

    useUpdates.getState().setAuto(false)
    expect(localStorage.getItem("autoscene.update.auto")).toBe("off")
    let stop = startUpdateSchedule()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(commands.updateCheck).not.toHaveBeenCalled()
    stop()

    useUpdates.getState().setAuto(true)
    expect(localStorage.getItem("autoscene.update.auto")).toBeNull()
    stop = startUpdateSchedule()
    await vi.advanceTimersByTimeAsync(10_000)
    expect(commands.updateCheck).toHaveBeenCalledTimes(1)
    stop()
  })

  it("link a version to its release page", () => {
    expect(releaseNotesUrl("1.0.3-BETA")).toBe("https://github.com/tcmmi/AutoScene/releases/tag/v1.0.3-BETA")
  })
})
