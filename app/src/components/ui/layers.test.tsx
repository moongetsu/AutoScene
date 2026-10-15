import { useState } from "react"
import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import { layerOpen, useLayerSafetyNet } from "@/lib/layers"
import { Dialog, DialogContent } from "./dialog"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./dropdown-menu"
import { Select } from "./select"

/**
 * Menus, selects and dialogs stack as layers: Escape closes only the top one,
 * and when the last one closes the window takes clicks again. Two copies of a
 * Radix primitive (see scripts/check-deps.mjs) once broke both: one Escape
 * closed a select and its dialog together and left `<body>` at
 * `pointer-events: none`, so nothing answered the mouse until a restart.
 */
afterEach(() => {
  cleanup()
  // A failing case must not leave the next one unclickable.
  document.body.style.pointerEvents = ""
})

describe("layers", () => {
  it("Escape in a select inside a dialog closes only the select", async () => {
    const user = userEvent.setup()
    render(<SelectInDialog />)
    screen.getByRole("combobox", { name: "Format" }).focus()
    await user.keyboard("{Enter}")
    expect(await screen.findByRole("listbox")).toBeTruthy()

    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull())
    expect(screen.getByRole("dialog")).toBeTruthy()

    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull())
    await waitFor(() => expect(document.body.style.pointerEvents).toBe(""))
    // user-event refuses to click through `pointer-events: none`.
    await user.click(screen.getByRole("button", { name: "Outside" }))
  })

  it("a dialog opened from a menu gives the window back when it closes", async () => {
    const user = userEvent.setup()
    render(<MenuThenDialog />)
    await user.click(screen.getByRole("button", { name: "Options" }))
    await user.click(await screen.findByRole("menuitem", { name: "Rename…" }))
    expect(await screen.findByRole("dialog")).toBeTruthy()

    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull())
    await waitFor(() => expect(document.body.style.pointerEvents).toBe(""))
    await user.click(screen.getByRole("button", { name: "Outside" }))
  })

  it("knows while a menu or a select is open", async () => {
    const user = userEvent.setup()
    render(<MenuThenDialog />)
    expect(layerOpen()).toBe(false)
    await user.click(screen.getByRole("button", { name: "Options" }))
    await screen.findByRole("menu")
    expect(layerOpen()).toBe(true)
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    expect(layerOpen()).toBe(false)

    cleanup()
    render(<SelectInDialog />)
    screen.getByRole("combobox", { name: "Format" }).focus()
    await user.keyboard("{Enter}")
    await screen.findByRole("listbox")
    expect(layerOpen()).toBe(true)
  })

  it("doesn't count a list that is part of the page", () => {
    render(<div role="listbox" aria-label="Results" />)
    expect(layerOpen()).toBe(false)
  })
})

describe("useLayerSafetyNet", () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  function Net() {
    useLayerSafetyNet()
    return null
  }

  it("gives the window back when it is left unclickable with nothing open", async () => {
    vi.useFakeTimers()
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {})
    render(<Net />)
    await act(async () => {
      document.body.style.pointerEvents = "none"
    })
    await act(async () => {
      vi.advanceTimersByTime(400)
    })
    expect(document.body.style.pointerEvents).toBe("")
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it("leaves an open dialog alone", async () => {
    vi.useFakeTimers()
    render(
      <>
        <Net />
        <div role="dialog" data-state="open" />
      </>,
    )
    await act(async () => {
      document.body.style.pointerEvents = "none"
    })
    await act(async () => {
      vi.advanceTimersByTime(400)
    })
    expect(document.body.style.pointerEvents).toBe("none")
  })
})

function SelectInDialog() {
  const [open, setOpen] = useState(true)
  const [format, setFormat] = useState("prores")
  return (
    <>
      <button type="button">Outside</button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title="Export">
          <Select
            aria-label="Format"
            value={format}
            onChange={setFormat}
            options={[
              { value: "prores", label: "ProRes 422 HQ" },
              { value: "h264", label: "H.264" },
            ]}
          />
        </DialogContent>
      </Dialog>
    </>
  )
}

function MenuThenDialog() {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button type="button">Outside</button>
      <DropdownMenu>
        <DropdownMenuTrigger>Options</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={() => setOpen(true)}>Rename…</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent title="Rename">A new name</DialogContent>
      </Dialog>
    </>
  )
}
