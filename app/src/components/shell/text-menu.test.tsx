import { useState } from "react"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import { useTextMenu } from "@/state/text-menu"
import { AppMenu } from "./AppMenu"
import { TextMenu } from "./TextMenu"

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => vi.fn(),
  useCanGoBack: () => false,
}))

afterEach(() => {
  act(() => useTextMenu.getState().close())
  cleanup()
  document.body.style.pointerEvents = ""
})

/** A rename dialog over the window, its field focused with "Walter" selected. */
function renderRename(onOpenChange = vi.fn()) {
  render(
    <AppMenu>
      <div>
        <Dialog open onOpenChange={onOpenChange}>
          <DialogContent title="Rename">
            <Field />
          </DialogContent>
        </Dialog>
        <TextMenu />
      </div>
    </AppMenu>,
  )
  const input = screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement
  input.focus()
  input.setSelectionRange(0, 6)
  return input
}

function Field() {
  const [name, setName] = useState("Walter White")
  return <input aria-label="Name" value={name} onChange={(e) => setName(e.target.value)} />
}

async function openOn(input: HTMLInputElement) {
  fireEvent.contextMenu(input, { clientX: 30, clientY: 30 })
  return screen.findByRole("menu", { name: "Edit" })
}

describe("the text fields' menu", () => {
  it("opens in the field's place without taking its focus or selection", async () => {
    const input = renderRename()
    await openOn(input)
    expect(document.activeElement).toBe(input)
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 6])
    // The window's menu stays shut.
    expect(screen.queryAllByRole("menu")).toHaveLength(1)
  })

  it("offers what can be done to the field as it is", async () => {
    const input = renderRename()
    await openOn(input)
    const state = (name: RegExp) => screen.getByRole("menuitem", { name }).getAttribute("aria-disabled")
    // Nothing typed yet, nothing on the clipboard (not in a test).
    expect(state(/^Undo/)).toBe("true")
    expect(state(/^Paste/)).toBe("true")
    expect(state(/^Cut/)).toBeNull()
    expect(state(/^Copy/)).toBeNull()
    expect(state(/^Delete/)).toBeNull()
    expect(state(/^Select all/)).toBeNull()
  })

  it("is worked with the keys while the field keeps the focus", async () => {
    const user = userEvent.setup()
    const input = renderRename()
    await openOn(input)
    await user.keyboard("{End}{Enter}")
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Edit" })).toBeNull())
    expect(document.activeElement).toBe(input)
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 12])
  })

  it("Delete removes the selection as typing would (React hears it)", async () => {
    const user = userEvent.setup()
    const input = renderRename()
    await openOn(input)
    await user.click(screen.getByRole("menuitem", { name: /^Delete/ }))
    expect(input.value).toBe(" White")
    expect(document.activeElement).toBe(input)
  })

  it("Escape closes the menu, not the dialog under it", async () => {
    const user = userEvent.setup()
    const onOpenChange = vi.fn()
    const input = renderRename(onOpenChange)
    await openOn(input)
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Edit" })).toBeNull())
    expect(onOpenChange).not.toHaveBeenCalled()
    expect(screen.getByRole("dialog")).toBeTruthy()
    expect(document.activeElement).toBe(input)
  })

  it("a click elsewhere closes it and the field keeps the focus", async () => {
    const input = renderRename()
    await openOn(input)
    // The layer starts listening for outside clicks a tick after it opens.
    await act(() => new Promise((r) => setTimeout(r, 0)))
    fireEvent.pointerDown(document.body)
    await waitFor(() => expect(screen.queryByRole("menu", { name: "Edit" })).toBeNull())
    expect(document.activeElement).toBe(input)
  })
})
