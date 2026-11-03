import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { PluginInfo, PluginReview, PluginUpdate } from "@/bindings"
import { commands } from "@/lib/ipc"
import { pluginInfo } from "@/test/plugins"
import { InstallReviewDialog } from "./InstallReviewDialog"

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  commands: {
    pluginInstall: vi.fn(() => Promise.resolve(pluginInfo({ id: "dev.example.discord-notify", name: "Discord Notify" }))),
    pluginDiscard: vi.fn(() => Promise.resolve(null)),
    pluginsAllowUnreviewed: vi.fn((on: boolean) =>
      Promise.resolve({ libraryRoot: "C:\\Library", exportPresetId: "prores-hq", audioLanguage: "eng", allowUnreviewedPlugins: on }),
    ),
  },
}))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }))

const discord = pluginInfo({
  id: "dev.example.discord-notify",
  name: "Discord Notify",
  version: "0.3.1",
  author: "Example Dev",
  trust: "unreviewed",
  permissions: ["exports:read", "network"],
  networkHosts: ["discord.com"],
  exportActions: [{ id: "post", title: "Post to Discord", description: null, afterExport: false }],
})

const review = (o: Partial<PluginReview> = {}): PluginReview => ({
  ticketId: "t1",
  plugin: discord,
  update: null,
  needsUnreviewed: true,
  allowed: true,
  uvMissing: false,
  ...o,
})

function show(r: PluginReview) {
  const onClose = vi.fn()
  render(<InstallReviewDialog review={r} onClose={onClose} />)
  return onClose
}

const install = () => screen.getByRole("button", { name: "Install" }) as HTMLButtonElement

beforeEach(() => vi.clearAllMocks())
afterEach(cleanup)

describe("the install review", () => {
  it("needs the name typed for an unreviewed plugin", async () => {
    const onClose = show(review())
    expect(screen.getByText("Not reviewed")).toBeTruthy()
    expect(install().disabled).toBe(true)
    const name = screen.getByLabelText("Type Discord Notify to install it")
    fireEvent.change(name, { target: { value: "discord notify" } })
    expect(install().disabled).toBe(true)
    fireEvent.change(name, { target: { value: " Discord Notify " } })
    expect(install().disabled).toBe(false)
    fireEvent.click(install())
    await waitFor(() => expect(commands.pluginInstall).toHaveBeenCalledExactlyOnceWith("t1", true))
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
    expect(toast).toHaveBeenCalledWith("Installed Discord Notify")
    // Installed: the review isn't dropped.
    expect(commands.pluginDiscard).not.toHaveBeenCalled()
    cleanup()

    // The setting off: it is turned on from here, and the review stays.
    show(review({ allowed: false }))
    expect(screen.queryByRole("textbox")).toBeNull()
    expect(install().disabled).toBe(true)
    vi.mocked(commands.pluginsAllowUnreviewed).mockImplementationOnce((on) =>
      Promise.resolve({ libraryRoot: "C:\\Library", exportPresetId: "prores-hq", audioLanguage: "eng", allowUnreviewedPlugins: !on }),
    )
    // The person said no in AutoScene's own dialog: still off.
    fireEvent.click(screen.getByRole("button", { name: "Allow them…" }))
    await waitFor(() => expect(commands.pluginsAllowUnreviewed).toHaveBeenCalledExactlyOnceWith(true))
    expect(screen.queryByRole("textbox")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Allow them…" }))
    fireEvent.change(await screen.findByLabelText("Type Discord Notify to install it"), { target: { value: "Discord Notify" } })
    expect(install().disabled).toBe(false)
    expect(commands.pluginDiscard).not.toHaveBeenCalled()
    cleanup()

    // A reviewed one installs at once, without the setting's yes.
    show(review({ plugin: { ...discord, trust: "reviewed" }, needsUnreviewed: false, allowed: true }))
    expect(screen.queryByRole("textbox")).toBeNull()
    fireEvent.click(install())
    await waitFor(() => expect(commands.pluginInstall).toHaveBeenLastCalledWith("t1", false))
  })

  it("says permissions are declared, not enforced", () => {
    show(review({ plugin: { ...discord, description: "Posts each export to Discord." } }))
    // What counts first, as plainly as the rest.
    expect(screen.getByText("It runs with your user rights: it can read, change and send anything you can.")).toBeTruthy()
    expect(screen.getByRole("heading", { name: "What it says it does (not enforced)" })).toBeTruthy()
    expect(screen.getByText("Reads the exported clips")).toBeTruthy()
    expect(screen.getByText("Talks to the internet")).toBeTruthy()
    expect(screen.getByText("discord.com")).toBeTruthy()
    // Who it is, as it says, and what it adds; its description as its own words, not the dialog's.
    expect(screen.getByText("dev.example.discord-notify")).toBeTruthy()
    expect(screen.getByText("0.3.1")).toBeTruthy()
    expect(screen.getByText("Says it is by")).toBeTruthy()
    expect(screen.getByText("Example Dev")).toBeTruthy()
    expect(screen.getByRole("heading", { name: "In its own words" })).toBeTruthy()
    expect(screen.getByRole("dialog").getAttribute("aria-describedby")).not.toBe(screen.getByText("Posts each export to Discord.").id)
    expect(screen.getByText("Post to Discord")).toBeTruthy()
    expect(screen.queryByText(/graphics card/)).toBeNull()
    cleanup()

    show(review({ plugin: { ...discord, permissions: [], networkHosts: [] } }))
    expect(screen.getByText("It names no permissions.")).toBeTruthy()
    cleanup()

    // The GPU, and what its plugin.json had skipped.
    show(review({ plugin: { ...discord, gpu: true, warnings: ["Skipped its page: AutoScene 1.0 has no plugin pages."] } }))
    expect(screen.getByText("Uses the graphics card: its runs wait while an analysis runs")).toBeTruthy()
    expect(screen.getByText("Skipped its page: AutoScene 1.0 has no plugin pages.")).toBeTruthy()
  })

  it("loads a folder where it is", () => {
    const folder = String.raw`D:\Plugins\discord-notify`
    show(review({ plugin: { ...discord, source: "folder", folder } }))
    expect(screen.getByRole("heading", { name: "Load Discord Notify?" })).toBeTruthy()
    expect(screen.getByText("Loaded from a folder")).toBeTruthy()
    expect(screen.getByText(folder)).toBeTruthy()
    expect(screen.getByLabelText("Type Discord Notify to load it")).toBeTruthy()
    cleanup()

    // A package's folder is AutoScene's own: not shown.
    show(review())
    expect(screen.queryByText(discord.folder)).toBeNull()
  })

  it("says what the installed version was when that changes", () => {
    const update: PluginUpdate = { fromVersion: "0.2.0", trustBefore: "reviewed", sourceBefore: "package", addedPermissions: [], addedHosts: [] }
    show(review({ update }))
    expect(screen.getByText("The installed version: Reviewed by AutoScene.")).toBeTruthy()
    cleanup()

    // A package in place of a developer's folder.
    show(review({ update: { ...update, trustBefore: "unreviewed", sourceBefore: "folder" } }))
    expect(
      screen.getByText("The installed version: Loaded from a folder. This package takes its place; that folder stays as it is."),
    ).toBeTruthy()
    cleanup()

    // Nothing to say when it stays as it was.
    show(review({ update: { ...update, trustBefore: "unreviewed" } }))
    expect(screen.queryByText(/The installed version/)).toBeNull()
  })

  it("takes a long name typed without its ellipsis", () => {
    const name = "A plugin whose name is far too long to …"
    show(review({ plugin: { ...discord, name } }))
    const field = screen.getByRole("textbox")
    for (const typed of ["A plugin whose name is far too long to", "A plugin whose name is far too long to...", name]) {
      fireEvent.change(field, { target: { value: typed } })
      expect(install().disabled, typed).toBe(false)
    }
    fireEvent.change(field, { target: { value: "A plugin whose name" } })
    expect(install().disabled).toBe(true)
    cleanup()

    // Typing nothing never names a plugin, whatever its name.
    show(review({ plugin: { ...discord, name: "..." } }))
    fireEvent.change(screen.getByRole("textbox"), { target: { value: " " } })
    expect(install().disabled).toBe(true)
  })

  it("shows the permissions an update adds", () => {
    const signed = { ...discord, trust: "pinned" as const, authorKeyId: "4B2D9C1E0F3A7E51", networkHosts: ["discord.com", "cdn.example.com"] }
    show(
      review({
        plugin: signed,
        update: {
          fromVersion: "0.2.0",
          trustBefore: "pinned",
          sourceBefore: "package",
          addedPermissions: ["network"],
          addedHosts: ["cdn.example.com"],
        },
      }),
    )
    expect(screen.getByRole("heading", { name: "Update Discord Notify?" })).toBeTruthy()
    expect(screen.getByText(/0\.2\.0 is installed/)).toBeTruthy()
    expect(screen.getByText("4B2D9C1E0F3A7E51")).toBeTruthy()
    expect(screen.getByText("Signed, not reviewed")).toBeTruthy()
    expect(screen.getByText("Signed with the same key as the installed version. AutoScene has not reviewed it.")).toBeTruthy()
    const network = screen.getByText("Talks to the internet").closest("li")!
    expect(within(network).getByText("New")).toBeTruthy()
    expect(within(network).getByText("New: cdn.example.com")).toBeTruthy()
    expect(within(network).getByText("discord.com")).toBeTruthy()
    // What the installed version could do already isn't marked.
    expect(within(screen.getByText("Reads the exported clips").closest("li")!).queryByText("New")).toBeNull()
    expect(screen.getByRole("button", { name: "Update" })).toBeTruthy()
  })

  it("keeps the review when the person says no outside the page", async () => {
    vi.mocked(commands.pluginInstall).mockRejectedValueOnce({ code: "cancelled", message: "It wasn't installed." })
    const onClose = show(review())
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Discord Notify" } })
    fireEvent.click(install())
    await waitFor(() => expect(install().disabled).toBe(false))
    expect(onClose).not.toHaveBeenCalled()
    expect(toast.error).not.toHaveBeenCalled()
    expect(commands.pluginDiscard).not.toHaveBeenCalled()
  })

  it("says how it runs, and that uv is missing", () => {
    show(review())
    expect(screen.getByText(/A Python script that uv runs\. On its first run, uv downloads Python/)).toBeTruthy()
    expect(screen.queryByText(/uv isn't installed/)).toBeNull()
    cleanup()

    show(review({ uvMissing: true }))
    expect(screen.getByText("uv isn't installed, so it can't run yet. Install it from docs.astral.sh/uv.")).toBeTruthy()
    cleanup()

    show(review({ plugin: { ...discord, uvScript: false } }))
    expect(screen.getByText("A program in its folder.")).toBeTruthy()
  })

  it("never says a first signature shows who made it", () => {
    const signed = { ...discord, trust: "pinned" as const, author: "AutoScene", authorKeyId: "4B2D9C1E0F3A7E51" }
    show(review({ plugin: signed }))
    const badge = screen.getByText("Signed, not reviewed")
    expect(badge.className).not.toContain("accent")
    expect(screen.getByText(/Signed with a key that came in the package: nothing shows who holds it\./)).toBeTruthy()
    // The name it gives its author is its own claim.
    expect(screen.getByText("Says it is by")).toBeTruthy()
    expect(screen.queryByText("Author")).toBeNull()
    cleanup()

    // An unsigned plugin's signed update pins whatever key it brings: said plainly.
    const update: PluginUpdate = { fromVersion: "0.2.0", trustBefore: "unreviewed", sourceBefore: "package", addedPermissions: [], addedHosts: [] }
    show(review({ plugin: signed, update }))
    expect(
      screen.getByText("The installed version wasn't signed. This one pins its key: from now on, only updates signed with it install."),
    ).toBeTruthy()
    cleanup()

    // Only a reviewed package's author is AutoScene's word.
    show(review({ plugin: { ...discord, trust: "reviewed" }, needsUnreviewed: false }))
    expect(screen.getByText("Author")).toBeTruthy()
  })

  it("drops the review when it is closed", async () => {
    const onClose = show(review())
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(commands.pluginDiscard).toHaveBeenCalledExactlyOnceWith("t1")
    expect(onClose).toHaveBeenCalledOnce()
    cleanup()

    // A refusal ends the review, but one while the plugin runs keeps it for another try.
    const reviewed = review({ needsUnreviewed: false, plugin: { ...discord, trust: "reviewed" } })
    vi.mocked(commands.pluginInstall).mockRejectedValueOnce({ code: "not_ready", message: "Discord Notify is running." })
    const kept = show(reviewed)
    fireEvent.click(install())
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("Couldn't install the plugin", { description: "Discord Notify is running." }))
    expect(kept).not.toHaveBeenCalled()
    expect(install().disabled).toBe(false)
    expect(commands.pluginDiscard).toHaveBeenCalledOnce()
    cleanup()

    vi.mocked(commands.pluginInstall).mockRejectedValueOnce({ code: "invalid", message: "This plugin needs another AutoScene." })
    const ended = show(reviewed)
    fireEvent.click(install())
    await waitFor(() => expect(ended).toHaveBeenCalledOnce())
    expect(toast.error).toHaveBeenLastCalledWith("Couldn't install the plugin", { description: "This plugin needs another AutoScene." })
    expect(commands.pluginDiscard).toHaveBeenLastCalledWith("t1")
    expect(commands.pluginDiscard).toHaveBeenCalledTimes(2)
  })

  it("can't be closed while it installs", async () => {
    let done: (p: PluginInfo) => void = () => {}
    vi.mocked(commands.pluginInstall).mockReturnValueOnce(new Promise((resolve) => (done = resolve)))
    const onClose = show(review({ needsUnreviewed: false, plugin: { ...discord, trust: "reviewed" } }))
    fireEvent.click(install())
    await waitFor(() => expect(install().disabled).toBe(true))
    expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" })
    fireEvent.click(screen.getByRole("button", { name: "Close" }))
    expect(screen.getByRole("dialog")).toBeTruthy()
    expect(onClose).not.toHaveBeenCalled()
    // It installs all the same, and then it closes, its review taken.
    done(discord)
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce())
    expect(commands.pluginDiscard).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledWith("Installed Discord Notify")
  })
})
