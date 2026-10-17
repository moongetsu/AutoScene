import { useEffect, useRef, useState } from "react"
import { act, cleanup, createEvent, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import { AppMenu } from "./AppMenu"

// The app's entries navigate; here only their order matters.
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => vi.fn(),
  useCanGoBack: () => false,
}))

afterEach(() => {
  cleanup()
  window.getSelection()?.removeAllRanges()
  // A failing case must not leave the next one unclickable.
  document.body.style.pointerEvents = ""
})

const SHORT_TAIL = ["Command palette…", "Settings"]
const FULL_LIST = [
  "Back",
  "Command palette…",
  "New project…",
  "Library",
  "Activity",
  "Hide the rail",
  "Keyboard shortcuts",
  "Settings",
  "Reload window",
]

/** The open menu's entries in order, as they read without their shortcuts. */
function entries(): string[] {
  return within(screen.getByRole("menu"))
    .getAllByRole("menuitem")
    .map((item) => {
      const label = item.cloneNode(true) as HTMLElement
      for (const keys of label.querySelectorAll(".timecode")) keys.remove()
      return label.textContent?.trim() ?? ""
    })
}

async function rightClick(el: Element) {
  fireEvent.contextMenu(el, { clientX: 40, clientY: 40 })
  await screen.findByRole("menu")
}

describe("the window's menu", () => {
  it("puts the element's own entries first, the innermost first, the short tail last", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea menu={<ContextMenuItem>Open project</ContextMenuItem>}>
            <section>
              <ContextArea menu={<ContextMenuItem>Watch shot</ContextMenuItem>}>
                <button>Shot</button>
              </ContextArea>
            </section>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByRole("button", { name: "Shot" }))
    expect(entries()).toEqual(["Watch shot", "Open project", ...SHORT_TAIL])
  })

  it("on a page's empty space: the page's entries, then all of the app's", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea kind="page" menu={<ContextMenuItem>Add files…</ContextMenuItem>}>
            <main>
              <p>Nothing here</p>
            </main>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByText("Nothing here"))
    expect(entries()).toEqual(["Add files…", ...FULL_LIST])
  })

  it("on an element of a page: the element's entries and the short tail, not the page's", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea kind="page" menu={<ContextMenuItem>Add files…</ContextMenuItem>}>
            <main>
              <ContextArea menu={<ContextMenuItem>Open</ContextMenuItem>}>
                <button>Episode</button>
              </ContextArea>
            </main>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByRole("button", { name: "Episode" }))
    expect(entries()).toEqual(["Open", ...SHORT_TAIL])
  })

  it("shows the full list where nothing says what it offers", async () => {
    render(
      <AppMenu>
        <div>
          <p>Rail</p>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByText("Rail"))
    expect(entries()).toEqual(FULL_LIST)
  })

  it("leaves the app's entries out under an element that says so", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea tail={false} menu={<ContextMenuItem>Minimize</ContextMenuItem>}>
            <button>Window</button>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByRole("button", { name: "Window" }))
    expect(entries()).toEqual(["Minimize"])
  })

  it("passes over an area with nothing to offer right now", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea menu={<ContextMenuItem>Open project</ContextMenuItem>}>
            <section>
              <ContextArea disabled menu={<ContextMenuItem>Busy</ContextMenuItem>}>
                <button>Card</button>
              </ContextArea>
            </section>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByRole("button", { name: "Card" }))
    expect(entries()).toEqual(["Open project", ...SHORT_TAIL])
  })

  it("offers Copy first for text selected in a selectable block", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea menu={<ContextMenuItem>Show in folder</ContextMenuItem>}>
            <p data-selectable>D:/Shows/Breaking Bad</p>
          </ContextArea>
          <p>Elsewhere</p>
        </div>
      </AppMenu>,
    )
    const path = screen.getByText("D:/Shows/Breaking Bad")
    const range = document.createRange()
    range.selectNodeContents(path)
    window.getSelection()?.addRange(range)
    await rightClick(path)
    expect(entries()).toEqual(["Copy", "Show in folder", ...SHORT_TAIL])
  })

  it("doesn't offer Copy for a selection the click wasn't on", async () => {
    render(
      <AppMenu>
        <div>
          <p data-selectable>D:/Shows/Breaking Bad</p>
          <p data-selectable>Elsewhere</p>
        </div>
      </AppMenu>,
    )
    const range = document.createRange()
    range.selectNodeContents(screen.getByText("D:/Shows/Breaking Bad"))
    window.getSelection()?.addRange(range)
    await rightClick(screen.getByText("Elsewhere"))
    expect(entries()).toEqual(FULL_LIST)
  })

  it("keeps an open menu up to date with its element", async () => {
    render(
      <AppMenu>
        <div>
          <Counter />
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByText("Faces"))
    expect(entries()[0]).toBe("Move 0 faces to…")
    // The window is inert under the menu; a click event still reaches the button.
    fireEvent.click(screen.getByText("One more"))
    expect(entries()[0]).toBe("Move 1 faces to…")
  })

  it("runs the entry chosen and closes", async () => {
    const user = userEvent.setup()
    const rename = vi.fn()
    render(
      <AppMenu>
        <div>
          <ContextArea menu={<ContextMenuItem onSelect={rename}>Rename…</ContextMenuItem>}>
            <button>Project</button>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByRole("button", { name: "Project" }))
    await user.click(screen.getByRole("menuitem", { name: "Rename…" }))
    expect(rename).toHaveBeenCalledOnce()
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
  })

  it("in a dialog: only what the dialog's areas offer, without the app's entries", async () => {
    render(
      <AppMenu>
        <div>
          <Dialog open>
            <DialogContent title="Export">
              <ContextArea menu={<ContextMenuItem>Copy file path</ContextMenuItem>}>
                <p>Destination</p>
              </ContextArea>
            </DialogContent>
          </Dialog>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByText("Destination"))
    expect(entries()).toEqual(["Copy file path"])
  })

  it("opens nothing in a dialog where nothing is offered", () => {
    render(
      <AppMenu>
        <div>
          <Dialog open>
            <DialogContent title="Export">
              <p>Just words</p>
            </DialogContent>
          </Dialog>
        </div>
      </AppMenu>,
    )
    const e = createEvent.contextMenu(screen.getByText("Just words"))
    fireEvent(screen.getByText("Just words"), e)
    expect(e.defaultPrevented).toBe(true)
    expect(screen.queryByRole("menu")).toBeNull()
  })

  it("in a dialog opened inside an element or a page: not their entries", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea kind="page" menu={<ContextMenuItem>Export the pack…</ContextMenuItem>}>
            <main>
              <ContextArea menu={<ContextMenuItem>Open project</ContextMenuItem>}>
                <section>
                  <Dialog open>
                    <DialogContent title="Export">
                      <p>Just words</p>
                      <ContextArea menu={<ContextMenuItem>Copy file path</ContextMenuItem>}>
                        <p>Destination</p>
                      </ContextArea>
                    </DialogContent>
                  </Dialog>
                </section>
              </ContextArea>
            </main>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    const e = createEvent.contextMenu(screen.getByText("Just words"))
    fireEvent(screen.getByText("Just words"), e)
    expect(e.defaultPrevented).toBe(true)
    expect(screen.queryByRole("menu")).toBeNull()
    await rightClick(screen.getByText("Destination"))
    expect(entries()).toEqual(["Copy file path"])
  })

  it("opens no empty menu in a dialog whose areas offer nothing right now", () => {
    const nothing = false
    render(
      <AppMenu>
        <div>
          <Dialog open>
            <DialogContent title="Activity">
              <ContextArea
                menu={
                  <>
                    {nothing && <ContextMenuItem>Clear finished</ContextMenuItem>}
                    <ContextMenuSeparator />
                  </>
                }
              >
                <p>In progress</p>
              </ContextArea>
            </DialogContent>
          </Dialog>
        </div>
      </AppMenu>,
    )
    fireEvent.contextMenu(screen.getByText("In progress"))
    expect(screen.queryByRole("menu")).toBeNull()
  })

  it("offers an area only where its `only` lets it (the top bar's empty parts)", async () => {
    render(
      <AppMenu>
        <div>
          <ContextArea tail={false} only={(t) => !t.closest("button")} menu={<ContextMenuItem>Minimize</ContextMenuItem>}>
            <header>
              <h1>Breaking Bad</h1>
              <button>Details</button>
            </header>
          </ContextArea>
        </div>
      </AppMenu>,
    )
    await rightClick(screen.getByText("Breaking Bad"))
    expect(entries()).toEqual(["Minimize"])
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" })
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    await rightClick(screen.getByText("Details"))
    expect(entries()).toEqual(FULL_LIST)
  })

  it("from the keyboard, gives the focus back — unless an entry took it", async () => {
    render(
      <AppMenu>
        <div>
          <Renamable />
        </div>
      </AppMenu>,
    )
    const project = screen.getByRole("button", { name: "Project" })
    // The menu key on a focused card.
    const fromKeyboard = async () => {
      project.focus()
      fireEvent.keyDown(window, { key: "ContextMenu" })
      await rightClick(project)
    }
    await fromKeyboard()
    fireEvent.click(screen.getByRole("menuitem", { name: "Copy name" }))
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    await act(() => new Promise((r) => setTimeout(r, 20)))
    expect(document.activeElement).toBe(project)

    await fromKeyboard()
    fireEvent.click(screen.getByRole("menuitem", { name: "Rename" }))
    const name = await screen.findByRole("textbox", { name: "Name" })
    await act(() => new Promise((r) => setTimeout(r, 20)))
    expect(document.activeElement).toBe(name)
  })

  it("never lets the browser's own menu show", () => {
    render(
      <AppMenu>
        <div>
          <p>Anywhere</p>
        </div>
      </AppMenu>,
    )
    const e = createEvent.contextMenu(document.body)
    fireEvent(document.body, e)
    expect(e.defaultPrevented).toBe(true)
  })

  it("while developing, Shift+right-click is the browser's (for Inspect)", () => {
    render(
      <AppMenu>
        <div>
          <p>Anywhere</p>
        </div>
      </AppMenu>,
    )
    const e = createEvent.contextMenu(screen.getByText("Anywhere"), { shiftKey: true })
    fireEvent(screen.getByText("Anywhere"), e)
    expect(e.defaultPrevented).toBe(import.meta.env.DEV ? false : true)
    if (import.meta.env.DEV) expect(screen.queryByRole("menu")).toBeNull()
  })
})

/** A card whose menu opens a name field in its place, as a character's hero does. */
function Renamable() {
  const [renaming, setRenaming] = useState(false)
  return (
    <>
      <ContextArea
        menu={
          <>
            <ContextMenuItem onSelect={() => setRenaming(true)}>Rename</ContextMenuItem>
            <ContextMenuItem>Copy name</ContextMenuItem>
          </>
        }
      >
        <button>Project</button>
      </ContextArea>
      {renaming && <NameField />}
    </>
  )
}

function NameField() {
  const ref = useRef<HTMLInputElement>(null)
  useEffect(() => ref.current?.focus(), [])
  return <input ref={ref} aria-label="Name" defaultValue="Breaking Bad" />
}

/** Faces and their selection: the menu speaks of what is selected now. */
function Counter() {
  const [n, setN] = useState(0)
  return (
    <>
      <ContextArea menu={<ContextMenuItem>Move {n} faces to…</ContextMenuItem>}>
        <span>Faces</span>
      </ContextArea>
      <button onClick={() => setN((x) => x + 1)}>One more</button>
    </>
  )
}
