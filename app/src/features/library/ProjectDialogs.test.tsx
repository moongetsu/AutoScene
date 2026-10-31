import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { ProjectSummary } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { ProjectDetailsDialog } from "./ProjectDialogs"

vi.mock("@/lib/ipc", () => ({
  commands: {
    projectSetContent: vi.fn((_id: string, content: string) => Promise.resolve({ content })),
    projectSetFastScan: vi.fn((_id: string, fastScan: boolean) => Promise.resolve({ fastScan })),
    projectRename: vi.fn(() => Promise.resolve({})),
  },
  errorMessage: (e: unknown) => String(e),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const film = {
  id: "p",
  title: "Film",
  kind: "movie",
  content: "live",
  fastScan: false,
} as ProjectSummary

function open(project: ProjectSummary, onOpenChange = vi.fn()) {
  render(
    <TooltipProvider>
      <ProjectDetailsDialog project={project} onOpenChange={onOpenChange} />
    </TooltipProvider>,
  )
  return onOpenChange
}

describe("ProjectDetailsDialog", () => {
  it("switches a project to Fast Scan and says its faces are found again", async () => {
    const user = userEvent.setup()
    const onOpenChange = open(film)
    expect(screen.getByText(/found in full/)).toBeTruthy()
    await user.click(screen.getByRole("switch", { name: "Fast Scan" }))
    expect(screen.getByText(/found again this way/)).toBeTruthy()
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
    expect(commands.projectSetFastScan).toHaveBeenCalledWith("p", true)
    expect(commands.projectSetContent).not.toHaveBeenCalled()
    expect(commands.projectRename).not.toHaveBeenCalled()
  })

  it("leaves the scan alone when only the title changes", async () => {
    const user = userEvent.setup()
    open({ ...film, fastScan: true })
    expect(screen.getByText(/the way the Scener app does/)).toBeTruthy()
    const title = screen.getByLabelText("Title")
    await user.clear(title)
    await user.type(title, "Film 2")
    await user.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(commands.projectRename).toHaveBeenCalledWith("p", "Film 2"))
    expect(commands.projectSetFastScan).not.toHaveBeenCalled()
  })

  it("has no Fast Scan for anime, which has its own pass", () => {
    open({ ...film, content: "anime" })
    expect(screen.queryByRole("switch", { name: "Fast Scan" })).toBeNull()
  })
})
