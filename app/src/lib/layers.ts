import { useEffect } from "react"

/** How long `<body>` may stay unclickable with nothing open before it is given back. */
const GRACE_MS = 300

/**
 * Last line of defence against a window that no longer takes clicks.
 *
 * Modal layers (dialogs, menus, selects) set `pointer-events: none` on
 * `<body>` while they are open and restore it when the last one closes. When
 * that bookkeeping goes wrong — two copies of a Radix primitive once did (see
 * scripts/check-deps.mjs) — nothing on screen answers the mouse again until a
 * restart. This gives the window back once nothing is open any more.
 */
export function useLayerSafetyNet() {
  useEffect(() => {
    let timer: number | undefined
    const check = () => {
      window.clearTimeout(timer)
      if (document.body.style.pointerEvents !== "none") return
      timer = window.setTimeout(() => {
        if (document.body.style.pointerEvents === "none" && !layerOpen()) {
          document.body.style.pointerEvents = ""
          console.warn("A closed layer left the window unclickable; it takes clicks again.")
        }
      }, GRACE_MS)
    }
    const observer = new MutationObserver(check)
    observer.observe(document.body, { attributes: true, attributeFilter: ["style"] })
    check()
    return () => {
      observer.disconnect()
      window.clearTimeout(timer)
    }
  }, [])
}

/** A dialog, sheet, menu, select or popover is on screen. */
function layerOpen(): boolean {
  return (
    document.querySelector(
      '[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"], [data-radix-popper-content-wrapper]',
    ) != null
  )
}
