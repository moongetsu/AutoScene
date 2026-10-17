/**
 * What the browser inside the window would still do on its own — the find
 * bar, printing, saving the page, reloading, caret browsing, a link's
 * download or new window, the middle button's autoscroll — stopped before it
 * shows. The app always goes first: a key reaches every handler of the app,
 * and only when none of them took it (`defaultPrevented`) is the browser's
 * own action cancelled. Editing keys (copy, cut, paste, undo, redo, select
 * all in a field) stay with the browser: they are the text fields' own.
 *
 * Zoom (Ctrl + wheel, Ctrl ±, pinch) and the browser's menu are already off
 * (Tauri's window settings; `AppMenu`).
 *
 * Not on macOS: there a ⌘ key the page cancels never reaches the app's menu
 * (⌘Q, ⌘W, ⌘H, the menu's ⌘K…), and WKWebView has no browser keys of its own.
 */
import { isMac } from "./platform"
import { isEditable } from "./shortcuts"

/** Editing keys with Ctrl / ⌘: the fields' own, never blocked. */
const EDITING = new Set(["c", "v", "x", "z", "y"])

/** Whether the browser would act on this key (and the app didn't ask for it). */
export function isBrowserKey(e: KeyboardEvent, dev: boolean): boolean {
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key
  // Function keys: find next (F3), caret browsing (F7), reload (F5).
  if (key === "F3" || key === "F7") return true
  if (key === "F5") return !dev
  const mod = e.ctrlKey || e.metaKey
  if (!mod || e.altKey) return false
  if (key.length !== 1 || !/[a-z0-9]/.test(key)) return false
  if (EDITING.has(key)) return false
  // Select all belongs to a field; outside one it would select the page.
  if (key === "a") return !isEditable(e.target)
  // Reload and the developer tools stay while developing.
  if (dev && (key === "r" || (e.shiftKey && ["i", "j", "c"].includes(key)))) return false
  return true
}

/** Clicks on a link that would make the browser download it or open a window. */
export function isBrowserLinkClick(e: MouseEvent): boolean {
  if (!(e.target instanceof Element) || !e.target.closest("a[href]")) return false
  return e.button !== 0 || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey
}

/** Install the guards on `win`; returns what removes them. */
export function installBrowserGuards({
  dev,
  mac = isMac,
  win = window,
}: {
  dev: boolean
  mac?: boolean
  win?: Window
}): () => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (!isBrowserKey(e, dev)) return
    // Runs after every other listener of this key (it is added last, and the
    // window's bubble listeners run after the page's): only a key nobody took
    // is the browser's — and then it is no one's.
    const last = (ev: Event) => {
      if (ev === e && !e.defaultPrevented) e.preventDefault()
    }
    win.addEventListener("keydown", last, { once: true })
  }
  const onLinkClick = (e: MouseEvent) => {
    if (isBrowserLinkClick(e)) e.preventDefault()
  }
  // The middle button's autoscroll compass.
  const onMouseDown = (e: MouseEvent) => {
    if (e.button === 1) e.preventDefault()
  }
  if (!mac) win.addEventListener("keydown", onKeyDown, { capture: true })
  win.addEventListener("click", onLinkClick, { capture: true })
  win.addEventListener("auxclick", onLinkClick, { capture: true })
  win.addEventListener("mousedown", onMouseDown, { capture: true })
  return () => {
    win.removeEventListener("keydown", onKeyDown, { capture: true })
    win.removeEventListener("click", onLinkClick, { capture: true })
    win.removeEventListener("auxclick", onLinkClick, { capture: true })
    win.removeEventListener("mousedown", onMouseDown, { capture: true })
  }
}
