import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { PluginInfo } from "@/bindings"
import { useUi } from "@/state/ui"
import { pluginInfo } from "@/test/plugins"
import { CommandPalette } from "./CommandPalette"

const { at, runPlugin } = vi.hoisted(() => ({ at: { path: "/", plugins: [] as PluginInfo[] }, runPlugin: vi.fn() }))

vi.mock("@tanstack/react-router", () => ({
  useRouterState: ({ select }: { select: (s: { location: { pathname: string } }) => unknown }) => select({ location: { pathname: at.path } }),
  useNavigate: () => vi.fn(),
}))
vi.mock("@/lib/queries", () => ({
  useProjects: () => ({ data: [] }),
  usePlugins: () => ({ data: at.plugins }),
  useCast: () => ({ data: undefined }),
  useMediaList: () => ({ data: undefined }),
}))
vi.mock("@/lib/actions", () => ({ useAppActions: () => ({ goSettings: vi.fn() }) }))
vi.mock("@/lib/shortcuts", () => ({ useShortcutKeys: () => () => [] }))
vi.mock("@/features/plugins/plugin-actions", () => ({ runPlugin }))

const command = (id: string, needsProject: boolean) => ({ id, title: `Run ${id}`, description: null, needsProject })

function open(path: string) {
  at.path = path
  useUi.setState({ paletteOpen: true })
  render(<CommandPalette />)
}

beforeEach(() => {
  vi.clearAllMocks()
  at.plugins = [pluginInfo({ id: "dev.a.one", name: "One", commands: [command("anywhere", false), command("here", true)] })]
})
afterEach(cleanup)

describe("the command palette's plugin commands", () => {
  it("run in the open project", async () => {
    open("/p/p1/exports")
    fireEvent.click(screen.getByRole("option", { name: /Run here/ }))
    await waitFor(() => expect(runPlugin).toHaveBeenCalledOnce())
    expect(runPlugin).toHaveBeenCalledWith(
      expect.objectContaining({ pluginId: "dev.a.one", actionId: "here", title: "Run here" }),
      "command",
      "p1",
      null,
    )
    cleanup()

    // Outside one, only those that need none, with no project.
    open("/settings/plugins")
    expect(screen.queryByRole("option", { name: /Run here/ })).toBeNull()
    fireEvent.click(screen.getByRole("option", { name: /Run anywhere/ }))
    await waitFor(() => expect(runPlugin).toHaveBeenCalledTimes(2))
    expect(runPlugin).toHaveBeenLastCalledWith(expect.objectContaining({ actionId: "anywhere" }), "command", null, null)
  })
})
