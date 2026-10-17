import { afterEach, beforeEach, describe, expect, it } from "vitest"
import { installBrowserGuards } from "./browser-guards"

let remove: () => void = () => {}

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init })
  target.dispatchEvent(e)
  return e
}

function click(target: EventTarget, init: MouseEventInit, type = "click"): MouseEvent {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, ...init })
  target.dispatchEvent(e)
  return e
}

describe("browser guards (release)", () => {
  beforeEach(() => {
    remove = installBrowserGuards({ dev: false })
  })
  afterEach(() => {
    remove()
    document.body.innerHTML = ""
  })

  it("stops the browser's find bar, printing, saving, caret browsing and reload", () => {
    for (const init of [
      { key: "f", ctrlKey: true },
      { key: "p", ctrlKey: true },
      { key: "s", ctrlKey: true },
      { key: "g", ctrlKey: true },
      { key: "r", ctrlKey: true },
      { key: "F3" },
      { key: "F5" },
      { key: "F7" },
      { key: "a", ctrlKey: true },
    ]) {
      expect(press(document.body, init).defaultPrevented, JSON.stringify(init)).toBe(true)
    }
  })

  it("leaves the editing keys and plain typing to text fields", () => {
    const input = document.createElement("input")
    document.body.append(input)
    for (const init of [
      { key: "c", ctrlKey: true },
      { key: "v", ctrlKey: true },
      { key: "x", ctrlKey: true },
      { key: "z", ctrlKey: true },
      { key: "y", ctrlKey: true },
      { key: "z", ctrlKey: true, shiftKey: true },
      { key: "a", ctrlKey: true },
      { key: "f" },
      { key: "Enter" },
    ]) {
      expect(press(input, init).defaultPrevented, JSON.stringify(init)).toBe(false)
    }
  })

  it("lets the app's own shortcut go first and see the key untouched", () => {
    let seen: boolean | null = null
    // The player's go-to (Ctrl+G): a listener added after the guards.
    const goTo = (e: KeyboardEvent) => {
      if (e.key === "g" && e.ctrlKey) {
        seen = e.defaultPrevented
        e.preventDefault()
      }
    }
    window.addEventListener("keydown", goTo)
    const e = press(document.body, { key: "g", ctrlKey: true })
    window.removeEventListener("keydown", goTo)
    expect(seen).toBe(false)
    expect(e.defaultPrevented).toBe(true)
  })

  it("stops a link's download, new window and the middle button's autoscroll", () => {
    const a = document.createElement("a")
    a.href = "#/p/1"
    a.textContent = "Breaking Bad"
    document.body.append(a)
    expect(click(a, { altKey: true }).defaultPrevented).toBe(true)
    expect(click(a, { ctrlKey: true }).defaultPrevented).toBe(true)
    expect(click(a, { button: 1 }, "auxclick").defaultPrevented).toBe(true)
    expect(click(a, {}).defaultPrevented).toBe(false)
    expect(click(document.body, { button: 1 }, "mousedown").defaultPrevented).toBe(true)
  })
})

describe("browser guards (development)", () => {
  afterEach(() => remove())

  it("keeps reload and the developer tools", () => {
    remove = installBrowserGuards({ dev: true })
    expect(press(document.body, { key: "F5" }).defaultPrevented).toBe(false)
    expect(press(document.body, { key: "r", ctrlKey: true }).defaultPrevented).toBe(false)
    expect(press(document.body, { key: "i", ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(false)
    expect(press(document.body, { key: "f", ctrlKey: true }).defaultPrevented).toBe(true)
  })
})

describe("browser guards (macOS)", () => {
  beforeEach(() => {
    remove = installBrowserGuards({ dev: false, mac: true })
  })
  afterEach(() => {
    remove()
    document.body.innerHTML = ""
  })

  it("leaves ⌘ keys to the app's menu (a key the page cancels never reaches it)", () => {
    for (const init of [
      { key: "q", metaKey: true },
      { key: "w", metaKey: true },
      { key: "k", metaKey: true },
      { key: "f", metaKey: true },
    ]) {
      expect(press(document.body, init).defaultPrevented).toBe(false)
    }
  })

  it("still keeps links from opening windows", () => {
    const link = document.createElement("a")
    link.href = "https://example.com"
    link.textContent = "Docs"
    document.body.append(link)
    expect(click(link, { metaKey: true }).defaultPrevented).toBe(true)
  })
})
