import { createContext, useContext } from "react"

export const PageScrolled = createContext(false)

/** Whether the page under the top bar has scrolled (the bar turns to glass). */
export function usePageScrolled(): boolean {
  return useContext(PageScrolled)
}

export const PageScrollElement = createContext<HTMLElement | null>(null)

/** The element the current page scrolls in (for lists that render only what is on screen). */
export function usePageScrollElement(): HTMLElement | null {
  return useContext(PageScrollElement)
}
