import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { ImportCandidate } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { useNewProject } from "@/state/new-project"
import { NewProjectDialog } from "./NewProjectDialog"

const episode = (n: number, refused: string | null = null): ImportCandidate => ({
  path: `D:\\Show\\Show - 0${n}.mkv`,
  fileName: `Show - 0${n}.mkv`,
  season: null,
  episode: n,
  label: `E0${n}`,
  sizeBytes: 1e9,
  duplicate: false,
  refused,
})

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }))
vi.mock("@/lib/ipc", () => ({
  commands: {
    importScan: vi.fn(() => Promise.resolve([episode(1), episode(2, "Its name isn't valid Unicode. Rename it to import it.")])),
    projectCreate: vi.fn(() => Promise.resolve({ id: "p" })),
  },
  errorMessage: (e: unknown) => String(e),
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  useNewProject.getState().setDragging(false)
  useNewProject.getState().setOpen(false)
})

describe("NewProjectDialog", () => {
  it("lists a file that can't be imported with why, and leaves it out", async () => {
    const user = userEvent.setup()
    useNewProject.getState().show(["D:\\Show"])
    render(
      <TooltipProvider>
        <NewProjectDialog />
      </TooltipProvider>,
    )
    expect(await screen.findByText(/isn't valid Unicode/)).toBeTruthy()
    expect(screen.getByText("1 video", { exact: false })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Skip Show - 02.mkv" })).toBeNull()
    await user.click(screen.getByRole("button", { name: "Create project" }))
    await waitFor(() => expect(commands.projectCreate).toHaveBeenCalled())
    expect(vi.mocked(commands.projectCreate).mock.calls[0][0].files).toEqual(["D:\\Show\\Show - 01.mkv"])
  })

  it("shows its file list as where dragged files go", () => {
    useNewProject.getState().show()
    render(
      <TooltipProvider>
        <NewProjectDialog />
      </TooltipProvider>,
    )
    expect(screen.queryByText("Drop to add to this project")).toBeNull()
    act(() => useNewProject.getState().setDragging(true))
    expect(screen.getByText("Drop to add to this project")).toBeTruthy()
    act(() => useNewProject.getState().setDragging(false))
    expect(screen.queryByText("Drop to add to this project")).toBeNull()
  })
})
