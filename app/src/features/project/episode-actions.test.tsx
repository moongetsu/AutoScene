import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { MediaItem } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { commands } from "@/lib/ipc"
import { EpisodeItems } from "./episode-actions"

vi.mock("@/lib/ipc", () => ({
  commands: { mediaRetry: vi.fn() },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { error: vi.fn() }) }))

/** The items as plain buttons. */
const kit: MenuKit = {
  Item: ({ onSelect, children }: { onSelect?: () => void; children: React.ReactNode }) => (
    <button onClick={onSelect}>{children}</button>
  ),
  Separator: () => null,
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("an episode's actions", () => {
  it("says so when indexing a failed episode again can't start", async () => {
    vi.mocked(commands.mediaRetry).mockRejectedValue("database is locked")
    const m = { id: "m", path: "C:\\Show\\E01.mkv", status: "failed", shots: "pending" } as MediaItem
    render(<EpisodeItems kit={kit} projectId="p" media={m} onOpen={() => {}} onRemove={() => {}} />)
    fireEvent.click(screen.getByRole("button", { name: /Index again/ }))
    expect(commands.mediaRetry).toHaveBeenCalledWith("p", "m")
    await vi.waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("Couldn't index it again", { description: "database is locked" }),
    )
  })
})
