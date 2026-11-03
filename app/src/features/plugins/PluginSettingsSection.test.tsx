import type * as React from "react"
import { QueryClientProvider } from "@tanstack/react-query"
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { PluginInfo, SettingSpec, SettingValue } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { queryClient } from "@/lib/queries"
import { pluginInfo } from "@/test/plugins"
import { PluginSettingsSection } from "./PluginSettingsSection"

const { state } = vi.hoisted(() => ({
  state: { plugin: null as PluginInfo | null, values: {} as Record<string, SettingValue> | Promise<Record<string, SettingValue>> },
}))

vi.mock("@/lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/ipc")>()),
  commands: {
    pluginsList: vi.fn(() => Promise.resolve(state.plugin ? [state.plugin] : [])),
    pluginSettingsGet: vi.fn(() => Promise.resolve(state.values)),
    pluginSettingsSet: vi.fn(async (_: string, v: Record<string, SettingValue>) => ({ ...(await state.values), ...v })),
    pluginSetAfterExport: vi.fn(),
  },
}))
vi.mock("@/lib/actions", () => ({ useAppActions: () => ({ goSettings: vi.fn() }) }))
vi.mock("sonner", () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }))
const { reveal } = vi.hoisted(() => ({ reveal: vi.fn() }))
vi.mock("@/lib/os", async (importOriginal) => ({ ...(await importOriginal<typeof import("@/lib/os")>()), reveal }))

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <QueryClientProvider client={queryClient}>
    <TooltipProvider>{children}</TooltipProvider>
  </QueryClientProvider>
)

const number = (key: string, title: string, o: { min?: number; max?: number; integer?: boolean }): SettingSpec => ({
  key,
  title,
  description: null,
  kind: { type: "number", default: null, min: o.min ?? null, max: o.max ?? null, step: null, integer: o.integer ?? false },
})

const select = (key: string, title: string, values: string[]): SettingSpec => ({
  key,
  title,
  description: null,
  kind: { type: "select", default: values[0], options: values.map((v) => ({ value: v, label: v })) },
})

function show(p: Partial<PluginInfo>, values: (typeof state)["values"] = {}) {
  state.plugin = pluginInfo({ id: "dev.a.hello", name: "Hello", ...p })
  state.values = values
  render(<PluginSettingsSection id="dev.a.hello" />, { wrapper })
}

const saved = () => vi.mocked(commands.pluginSettingsSet).mock.calls.map((c) => c[1])

beforeEach(() => {
  queryClient.clear()
  vi.clearAllMocks()
})
afterEach(cleanup)

describe("a plugin's settings", () => {
  it("shows its folder and its log", async () => {
    show({})
    await screen.findByText("Its folder")
    for (const button of screen.getAllByRole("button", { name: "Reveal" })) fireEvent.click(button)
    expect(reveal.mock.calls).toEqual([[state.plugin!.folder], [state.plugin!.logPath]])
  })

  it("saves a number within its bounds", async () => {
    show({ settings: [number("clips", "Clips", { min: 1, integer: true }), number("pause", "Pause", { max: 2 })] }, { clips: 3 })
    const clips = (await screen.findByRole("textbox", { name: "Clips" })) as HTMLInputElement
    await waitFor(() => expect(clips.value).toBe("3"))
    // Typing alone saves nothing; leaving the field does, kept within bounds and whole.
    fireEvent.change(clips, { target: { value: "0" } })
    expect(commands.pluginSettingsSet).not.toHaveBeenCalled()
    fireEvent.blur(clips)
    await waitFor(() => expect(saved()).toEqual([{ clips: 1 }]))
    fireEvent.change(clips, { target: { value: "4,6" } })
    fireEvent.keyDown(clips, { key: "Enter" })
    fireEvent.blur(clips)
    await waitFor(() => expect(saved()).toEqual([{ clips: 1 }, { clips: 5 }]))
    const pause = screen.getByRole("textbox", { name: "Pause" })
    fireEvent.change(pause, { target: { value: "7.5" } })
    fireEvent.blur(pause)
    await waitFor(() => expect(saved().at(-1)).toEqual({ pause: 2 }))
    // What isn't a number is never saved.
    fireEvent.change(pause, { target: { value: "lots" } })
    fireEvent.blur(pause)
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("Couldn't save the setting", { description: "Pause takes a number." }))
    expect(saved()).toHaveLength(3)
  })

  it("saves a slider once it rests", async () => {
    show({ settings: [{ ...number("level", "Level", { min: 0, max: 10, integer: true }) }] }, { level: 4 })
    const thumb = await screen.findByRole("slider", { name: "Level" })
    await waitFor(() => expect(thumb.getAttribute("aria-valuenow")).toBe("4"))
    fireEvent.keyDown(thumb, { key: "ArrowRight" })
    fireEvent.keyDown(thumb, { key: "ArrowRight" })
    expect(commands.pluginSettingsSet).not.toHaveBeenCalled()
    await waitFor(() => expect(saved()).toEqual([{ level: 6 }]))
  })

  it("saves a slider still moving when the page goes", async () => {
    show({ settings: [{ ...number("level", "Level", { min: 0, max: 10, integer: true }) }] }, { level: 4 })
    const thumb = await screen.findByRole("slider", { name: "Level" })
    await waitFor(() => expect(thumb.getAttribute("aria-valuenow")).toBe("4"))
    fireEvent.keyDown(thumb, { key: "ArrowRight" })
    cleanup()
    expect(saved()).toEqual([{ level: 5 }])
  })

  it("types a number that has no room for a slider", async () => {
    show({ settings: [number("fixed", "Fixed", { min: 3, max: 3 }), number("huge", "Huge", { min: -1e308, max: 1e308 })] }, { fixed: 3 })
    // Both bounds, but nothing between them, or more than a number holds.
    expect(await screen.findByRole("textbox", { name: "Fixed" })).toBeTruthy()
    expect(screen.getByRole("textbox", { name: "Huge" })).toBeTruthy()
    expect(screen.queryByRole("slider")).toBeNull()
  })

  it("saves each kind of setting", async () => {
    const user = userEvent.setup()
    const text: SettingSpec = {
      key: "greeting",
      title: "Greeting",
      description: null,
      kind: { type: "string", default: "Hi", placeholder: null, maxLength: 20 },
    }
    show({
      settings: [
        { key: "loud", title: "Loud", description: null, kind: { type: "boolean", default: false } },
        text,
        select("mood", "Mood", ["calm", "keen"]),
        select("colour", "Colour", ["red", "green", "blue", "gold"]),
      ],
    })
    const loud = (await screen.findByRole("switch", { name: "Loud" })) as HTMLButtonElement
    await waitFor(() => expect(loud.disabled).toBe(false))
    fireEvent.click(loud)
    await waitFor(() => expect(saved()).toEqual([{ loud: true }]))

    const greeting = screen.getByRole("textbox", { name: "Greeting" }) as HTMLInputElement
    expect(greeting.value).toBe("Hi")
    fireEvent.change(greeting, { target: { value: "Hello" } })
    fireEvent.blur(greeting)
    await waitFor(() => expect(saved().at(-1)).toEqual({ greeting: "Hello" }))

    // Two or three choices side by side, more in a list.
    fireEvent.click(within(screen.getByRole("radiogroup", { name: "Mood" })).getByRole("radio", { name: "keen" }))
    await waitFor(() => expect(saved().at(-1)).toEqual({ mood: "keen" }))
    screen.getByRole("combobox", { name: "Colour" }).focus()
    await user.keyboard("{Enter}")
    await user.click(await screen.findByRole("option", { name: "gold" }))
    await waitFor(() => expect(saved().at(-1)).toEqual({ colour: "gold" }))
    expect(saved()).toHaveLength(4)
  })

  it("waits for the stored values before a setting can change", async () => {
    let read: (values: Record<string, SettingValue>) => void = () => {}
    const stored = new Promise<Record<string, SettingValue>>((resolve) => (read = resolve))
    show({ settings: [number("level", "Level", { min: 0, max: 10, integer: true }), select("mood", "Mood", ["calm", "keen"])] }, stored)
    const thumb = await screen.findByRole("slider", { name: "Level" })
    const keen = within(screen.getByRole("radiogroup", { name: "Mood" })).getByRole("radio", { name: "keen" }) as HTMLButtonElement
    expect([thumb.hasAttribute("data-disabled"), keen.disabled]).toEqual([true, true])
    read({ level: 7, mood: "keen" })
    await waitFor(() => expect(thumb.getAttribute("aria-valuenow")).toBe("7"))
    expect([thumb.hasAttribute("data-disabled"), keen.disabled]).toEqual([false, false])
    expect(keen.getAttribute("aria-checked")).toBe("true")
  })

  it("says so when its settings can't be read", async () => {
    const full = { code: "io", message: "The disk is full." }
    // Tried twice.
    vi.mocked(commands.pluginSettingsGet).mockRejectedValueOnce(full).mockRejectedValueOnce(full)
    show({ settings: [select("mood", "Mood", ["calm", "keen"])] })
    expect(await screen.findByText("Couldn't read its settings: The disk is full.", undefined, { timeout: 5000 })).toBeTruthy()
    expect((screen.getByRole("radio", { name: "keen" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("reads only a setting's own value, never one its object inherits", async () => {
    const text = (key: string, title: string): SettingSpec => ({
      key,
      title,
      description: null,
      kind: { type: "string", default: null, placeholder: null, maxLength: 20 },
    })
    // A setting may be called constructor, or anything else an object inherits.
    const values = Object.create({ name: "inherited" }) as Record<string, SettingValue>
    show({ settings: [text("constructor", "Built by"), text("name", "Name")] }, values)
    const name = (await screen.findByRole("textbox", { name: "Name" })) as HTMLInputElement
    await waitFor(() => expect(name.disabled).toBe(false))
    expect(name.value).toBe("")
    expect((screen.getByRole("textbox", { name: "Built by" }) as HTMLInputElement).value).toBe("")
  })

  it("turns on Run after every export", async () => {
    const action = { id: "hello", title: "Say hello", description: null, afterExport: false }
    show({ exportActions: [action] })
    const on = await screen.findByRole("switch", { name: "Run Say hello after every export" })
    expect(on.getAttribute("aria-checked")).toBe("false")
    vi.mocked(commands.pluginSetAfterExport).mockResolvedValue({ ...state.plugin!, exportActions: [{ ...action, afterExport: true }] })
    fireEvent.click(on)
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Run Say hello after every export" }).getAttribute("aria-checked")).toBe("true"),
    )
    expect(commands.pluginSetAfterExport).toHaveBeenCalledExactlyOnceWith("dev.a.hello", "hello", true)
  })

  it("says so for a plugin that isn't installed", async () => {
    state.plugin = null
    render(<PluginSettingsSection id="dev.a.gone" />, { wrapper })
    expect(await screen.findByText("This plugin isn't installed")).toBeTruthy()
  })
})
