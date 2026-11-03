import type * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { AppSettings, PluginInfo, PluginReview } from "@/bindings"
import { AppMenu } from "@/components/shell/AppMenu"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { pluginInfo } from "@/test/plugins"
import { PluginsPage } from "./PluginsPage"

const { state, reveal } = vi.hoisted(() => ({ state: { plugins: [] as PluginInfo[], allow: false }, reveal: vi.fn() }))

const settings = (allow: boolean): AppSettings => ({
  libraryRoot: "C:\\Library",
  exportPresetId: "prores-hq",
  audioLanguage: "eng",
  allowUnreviewedPlugins: allow,
})

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  commands: {
    settingsGet: vi.fn(() => Promise.resolve(settings(state.allow))),
    pluginsList: vi.fn(() => Promise.resolve(state.plugins)),
    pluginsAllowUnreviewed: vi.fn((on: boolean) => Promise.resolve(settings(on))),
    pluginSetEnabled: vi.fn(),
    pluginUninstall: vi.fn(() => Promise.resolve(null)),
    pluginInspectFolder: vi.fn(),
    pluginDiscard: vi.fn(() => Promise.resolve(null)),
    revealPath: vi.fn(() => Promise.resolve()),
  },
}))
vi.mock("@/lib/media", () => ({ pickFolder: vi.fn(() => Promise.resolve("D:\\Plugins\\mine")), pickPluginFile: vi.fn() }))
vi.mock("@/lib/os", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/lib/os")>()), reveal }))
vi.mock("@/lib/actions", () => ({ useAppActions: () => ({ goSettings: vi.fn() }) }))
// The window's menu, around the rows' own entries.
vi.mock("@/components/ui/page", () => ({ Page: ({ children }: { children: React.ReactNode }) => <main>{children}</main> }))
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useCanGoBack: () => false,
  useRouterState: () => ({ location: { pathname: "/plugins" } }),
}))

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider>{children}</TooltipProvider>
  </QueryClientProvider>
)
const allowSwitch = () => screen.getByRole("switch", { name: "Allow plugins that AutoScene has not reviewed" })

beforeEach(() => {
  queryClient.clear()
  vi.clearAllMocks()
  state.plugins = []
  state.allow = false
})
afterEach(cleanup)

describe("PluginsPage", () => {
  it("lists plugins with their trust", async () => {
    state.plugins = [
      pluginInfo({ id: "dev.a.reviewed", name: "Reviewed One", description: "Says hello.", author: "AutoScene" }),
      pluginInfo({ id: "dev.a.pinned", name: "Pinned One", trust: "pinned", authorKeyId: "4B2D9C1E0F3A7E51" }),
      pluginInfo({
        id: "dev.a.unreviewed",
        name: "Unreviewed One",
        trust: "unreviewed",
        active: false,
        blockedReason: "Plugins that AutoScene has not reviewed are turned off in Settings.",
      }),
      // A folder says so first, whatever its trust.
      pluginInfo({ id: "dev.a.folder", name: "Folder One", trust: "unreviewed", source: "folder", enabled: false, active: false }),
    ]
    render(<PluginsPage />, { wrapper })
    const row = (name: string) => screen.findByRole("group", { name })
    const badges: [string, string][] = [
      ["Reviewed One", "Reviewed by AutoScene"],
      ["Pinned One", "Signed, not reviewed"],
      ["Unreviewed One", "Not reviewed"],
      ["Folder One", "Loaded from a folder"],
    ]
    for (const [name, badge] of badges) expect(within(await row(name)).getByText(badge)).toBeTruthy()
    expect(within(await row("Reviewed One")).getByText("By AutoScene, version 1.0.0")).toBeTruthy()
    // Only AutoScene's review stands behind the name a plugin gives its author.
    expect(within(await row("Pinned One")).getByText("Says it is by Someone, version 1.0.0")).toBeTruthy()
    // Why one that is turned on doesn't run; nothing for one turned off.
    expect(within(await row("Unreviewed One")).getByText(/turned off in Settings/)).toBeTruthy()
    const folder = screen.getByRole("switch", { name: "Enable Folder One" })
    expect(folder.getAttribute("aria-checked")).toBe("false")
    expect(screen.getByRole("switch", { name: "Enable Reviewed One" }).getAttribute("aria-checked")).toBe("true")

    // Turned on, the list shows what the backend gave back.
    vi.mocked(commands.pluginSetEnabled).mockResolvedValue({ ...state.plugins[3], enabled: true })
    fireEvent.click(folder)
    await waitFor(() => expect(screen.getByRole("switch", { name: "Enable Folder One" }).getAttribute("aria-checked")).toBe("true"))
    expect(commands.pluginSetEnabled).toHaveBeenCalledWith("dev.a.folder", true)
  })

  it("says so when no plugin is installed", async () => {
    render(<PluginsPage />, { wrapper })
    expect(await screen.findByText("No plugins yet")).toBeTruthy()
  })

  it("leaves the question about unreviewed plugins to AutoScene's own dialog", async () => {
    render(<PluginsPage />, { wrapper })
    await waitFor(() => expect((allowSwitch() as HTMLButtonElement).disabled).toBe(false))
    // The person said no there: it stays off.
    vi.mocked(commands.pluginsAllowUnreviewed).mockResolvedValueOnce(settings(false))
    fireEvent.click(allowSwitch())
    await waitFor(() => expect(commands.pluginsAllowUnreviewed).toHaveBeenCalledExactlyOnceWith(true))
    expect(screen.queryByRole("dialog")).toBeNull()
    expect(allowSwitch().getAttribute("aria-checked")).toBe("false")

    fireEvent.click(allowSwitch())
    await waitFor(() => expect(allowSwitch().getAttribute("aria-checked")).toBe("true"))
    // Through its own command, never a settings save.
    expect(commands.pluginsAllowUnreviewed).toHaveBeenLastCalledWith(true)
    expect(queryClient.getQueryData<AppSettings>(qk.settings)?.allowUnreviewedPlugins).toBe(true)

    // Off needs no question.
    fireEvent.click(allowSwitch())
    await waitFor(() => expect(commands.pluginsAllowUnreviewed).toHaveBeenLastCalledWith(false))
    expect(screen.queryByRole("dialog")).toBeNull()
    await waitFor(() => expect(allowSwitch().getAttribute("aria-checked")).toBe("false"))
  })

  it("offers a plugin's actions in its ⋯ menu too", async () => {
    state.plugins = [pluginInfo({ id: "dev.a.one", name: "One" })]
    render(<PluginsPage />, { wrapper })
    const trigger = await screen.findByRole("button", { name: "One options" })
    fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false })
    const menu = await screen.findByRole("menu")
    expect(within(menu).getAllByRole("menuitem").map((i) => i.textContent?.trim())).toEqual([
      "Show its folder",
      "Show its log",
      "Uninstall…",
    ])
    fireEvent.click(within(menu).getByRole("menuitem", { name: "Uninstall…" }))
    expect(await screen.findByRole("dialog", { name: "Uninstall One?" })).toBeTruthy()
  })

  it("stays open while it uninstalls", async () => {
    state.plugins = [pluginInfo({ id: "dev.a.one", name: "One" })]
    let done: (v: null) => void = () => {}
    vi.mocked(commands.pluginUninstall).mockReturnValueOnce(new Promise((resolve) => (done = resolve)))
    render(<PluginsPage />, { wrapper })
    fireEvent.pointerDown(await screen.findByRole("button", { name: "One options" }), { button: 0, ctrlKey: false })
    fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Uninstall…" }))
    const dialog = await screen.findByRole("dialog")
    fireEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => expect((within(dialog).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true))
    fireEvent.keyDown(dialog, { key: "Escape" })
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }))
    expect(screen.getByRole("dialog")).toBeTruthy()
    done(null)
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull())
  })

  it("spins the button whose choice is being checked", async () => {
    state.allow = true
    let checked: (r: PluginReview) => void = () => {}
    vi.mocked(commands.pluginInspectFolder).mockReturnValueOnce(new Promise((resolve) => (checked = resolve)))
    render(<PluginsPage />, { wrapper })
    const load = await screen.findByRole("button", { name: /Load a folder/ })
    fireEvent.click(load)
    await waitFor(() => expect(load.querySelector(".animate-spin")).not.toBeNull())
    expect(screen.getByRole("button", { name: /Install a plugin/ }).querySelector(".animate-spin")).toBeNull()
    checked({ ticketId: "t", plugin: pluginInfo({ id: "dev.me.mine", name: "Mine" }), update: null, needsUnreviewed: true, allowed: true, uvMissing: false })
    await waitFor(() => expect(load.querySelector(".animate-spin")).toBeNull())
  })

  it("uninstalls a plugin, and what it kept only when asked", async () => {
    state.plugins = [pluginInfo({ id: "dev.a.one", name: "One" }), pluginInfo({ id: "dev.a.two", name: "Two" })]
    render(
      <AppMenu>
        <div>
          <PluginsPage />
        </div>
      </AppMenu>,
      { wrapper },
    )
    const uninstall = async (name: string) => {
      fireEvent.contextMenu(await screen.findByRole("group", { name }), { clientX: 40, clientY: 40 })
      fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Uninstall…" }))
      return screen.findByRole("dialog")
    }
    // What it kept between runs stays, unless the person says otherwise.
    let dialog = await uninstall("One")
    expect(within(dialog).getByRole("switch", { name: "Delete what it kept too" }).getAttribute("aria-checked")).toBe("false")
    fireEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => expect(commands.pluginUninstall).toHaveBeenCalledExactlyOnceWith("dev.a.one", false))
    await waitFor(() => expect(screen.queryByRole("group", { name: "One" })).toBeNull())

    dialog = await uninstall("Two")
    fireEvent.click(within(dialog).getByRole("switch", { name: "Delete what it kept too" }))
    fireEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }))
    await waitFor(() => expect(commands.pluginUninstall).toHaveBeenLastCalledWith("dev.a.two", true))
  })

  it("hides Load a folder while unreviewed plugins are off", async () => {
    render(<PluginsPage />, { wrapper })
    expect(await screen.findByRole("button", { name: "Install a plugin…" })).toBeTruthy()
    await waitFor(() => expect((allowSwitch() as HTMLButtonElement).disabled).toBe(false))
    expect(screen.queryByRole("button", { name: "Load a folder…" })).toBeNull()
    cleanup()
    queryClient.clear()

    state.allow = true
    render(<PluginsPage />, { wrapper })
    expect(await screen.findByRole("button", { name: "Load a folder…" })).toBeTruthy()
  })

  it("filters plugins by search text and filter tabs", async () => {
    state.plugins = [
      pluginInfo({ id: "dev.a.alpha", name: "Alpha Tool", author: "Alice", description: "Video helper", enabled: true, gpu: true }),
      pluginInfo({ id: "dev.b.beta", name: "Beta Script", author: "Bob", description: "Audio cleaner", enabled: false, gpu: false }),
    ]
    render(<PluginsPage />, { wrapper })
    expect(await screen.findByRole("group", { name: "Alpha Tool" })).toBeTruthy()
    expect(screen.getByRole("group", { name: "Beta Script" })).toBeTruthy()

    // Search query
    const input = screen.getByPlaceholderText("Search plugins…")
    fireEvent.change(input, { target: { value: "alpha" } })
    expect(screen.getByRole("group", { name: "Alpha Tool" })).toBeTruthy()
    expect(screen.queryByRole("group", { name: "Beta Script" })).toBeNull()

    // Clear search
    fireEvent.change(input, { target: { value: "" } })
    expect(screen.getByRole("group", { name: "Alpha Tool" })).toBeTruthy()
    expect(screen.getByRole("group", { name: "Beta Script" })).toBeTruthy()

    // Filter tab: Disabled
    fireEvent.click(screen.getByRole("button", { name: /Disabled/ }))
    expect(screen.queryByRole("group", { name: "Alpha Tool" })).toBeNull()
    expect(screen.getByRole("group", { name: "Beta Script" })).toBeTruthy()

    // Filter tab: GPU
    fireEvent.click(screen.getByRole("button", { name: /GPU/ }))
    expect(screen.getByRole("group", { name: "Alpha Tool" })).toBeTruthy()
    expect(screen.queryByRole("group", { name: "Beta Script" })).toBeNull()
  })

  it("opens plugin folder and log via ⋯ menu items", async () => {
    const user = userEvent.setup()
    state.plugins = [pluginInfo({ id: "dev.a.alpha", name: "Alpha Tool", folder: "/path/to/folder", logPath: "/path/to/log.txt" })]
    render(<PluginsPage />, { wrapper })
    const trigger = await screen.findByRole("button", { name: "Alpha Tool options" })
    await user.click(trigger)
    await user.click(await screen.findByRole("menuitem", { name: "Show its folder" }))
    expect(reveal).toHaveBeenCalledWith("/path/to/folder")

    await user.click(trigger)
    await user.click(await screen.findByRole("menuitem", { name: "Show its log" }))
    expect(reveal).toHaveBeenCalledWith("/path/to/log.txt")
  })

  it("enables and disables all plugins via bulk action buttons", async () => {
    state.plugins = [
      pluginInfo({ id: "dev.a.alpha", name: "Alpha", enabled: false }),
      pluginInfo({ id: "dev.b.beta", name: "Beta", enabled: true }),
    ]
    vi.mocked(commands.pluginSetEnabled).mockImplementation((id, on) =>
      Promise.resolve(pluginInfo({ id, name: id, enabled: on })),
    )
    render(<PluginsPage />, { wrapper })

    // Enable all
    fireEvent.click(await screen.findByRole("button", { name: "Enable all" }))
    await waitFor(() => expect(commands.pluginSetEnabled).toHaveBeenCalledWith("dev.a.alpha", true))

    // Disable all
    fireEvent.click(screen.getByRole("button", { name: "Disable all" }))
    await waitFor(() => expect(commands.pluginSetEnabled).toHaveBeenCalledWith("dev.b.beta", false))
  })

  it("navigates to settings on clicking a card with settings page", async () => {
    state.plugins = [
      pluginInfo({
        id: "dev.a.alpha",
        name: "Alpha Tool",
        settings: [{ key: "opt", title: "Option", description: "Test desc", kind: { type: "boolean", default: false } }],
      }),
    ]
    render(<PluginsPage />, { wrapper })
    const card = await screen.findByRole("group", { name: "Alpha Tool" })
    expect(card.className).toContain("cursor-pointer")
  })
})
