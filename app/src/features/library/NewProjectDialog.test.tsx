import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { useNewProject } from "@/state/new-project"
import { NewProjectDialog } from "./NewProjectDialog"

vi.mock("@/lib/ipc", () => ({
  commands: { importScan: vi.fn().mockResolvedValue([]) },
  errorMessage: (e: unknown) => String(e),
}))
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }))

afterEach(() => {
  cleanup()
  act(() => {
    useNewProject.getState().setDragging(false)
    useNewProject.getState().setOpen(false)
  })
})

describe("the New Project dialog", () => {
  it("shows its file list as where dragged files go", () => {
    act(() => useNewProject.getState().show())
    render(<NewProjectDialog />)
    expect(screen.queryByText("Drop to add to this project")).toBeNull()

    act(() => useNewProject.getState().setDragging(true))
    expect(screen.getByText("Drop to add to this project")).toBeTruthy()

    act(() => useNewProject.getState().setDragging(false))
    expect(screen.queryByText("Drop to add to this project")).toBeNull()
  })
})
