import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { JobInfo, MediaItem } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { commands } from "@/lib/ipc"
import { useMediaJob } from "@/state/activity"
import { EpisodeItems } from "./episode-actions"

vi.mock("@/lib/ipc", () => ({
  commands: { mediaRetry: vi.fn(), jobCancel: vi.fn() },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { error: vi.fn() }) }))
vi.mock("@/state/activity", () => ({
  useMediaJob: vi.fn(),
}))

/** The items as plain buttons. */
const kit: MenuKit = {
  Item: ({ onSelect, disabled, children }: { onSelect?: () => void; disabled?: boolean; children: React.ReactNode }) => (
    <button disabled={disabled} onClick={onSelect}>{children}</button>
  ),
  Separator: () => null,
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("an episode's actions", () => {
  it("says so when indexing a failed episode again can't start", async () => {
    vi.mocked(useMediaJob).mockReturnValue(undefined)
    vi.mocked(commands.mediaRetry).mockRejectedValue("database is locked")
    const m = { id: "m", path: "C:\\Show\\E01.mkv", status: "failed", shots: "pending" } as MediaItem
    render(<EpisodeItems kit={kit} projectId="p" media={m} faces onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Index again/ }))
    expect(commands.mediaRetry).toHaveBeenCalledWith("p", "m")
    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Couldn't index it again", { description: "database is locked" }),
    )
  })

  it("offers cancelling and hides re-run options when a job is running", () => {
    vi.mocked(useMediaJob).mockReturnValue({
      id: "j1",
      kind: "analyze",
      label: "Analysis",
      state: "running",
      progress: 0.5,
    } as unknown as JobInfo)
    const m = { id: "m", path: "C:\\Show\\E01.mkv", status: "ready", shots: "ready" } as MediaItem
    render(<EpisodeItems kit={kit} projectId="p" media={m} faces onOpen={() => {}} onRemove={() => {}} />)

    expect(screen.getByRole("button", { name: /Cancel analysis/ })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /Find shots again/ })).toBeNull()
    expect(screen.queryByRole("button", { name: /Find characters again/ })).toBeNull()

    const removeBtn = screen.getByRole("button", { name: /Remove from project…/ }) as HTMLButtonElement
    expect(removeBtn.disabled).toBe(true)

    vi.mocked(commands.jobCancel).mockResolvedValue(undefined)
    fireEvent.click(screen.getByRole("button", { name: /Cancel analysis/ }))
    expect(commands.jobCancel).toHaveBeenCalledWith("j1")
  })
})
