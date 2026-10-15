import { createContext, useContext } from "react"

export const PageScrolled = createContext(false)

/** Whether the page under the top bar has scrolled (the bar turns to glass). */
export function usePageScrolled(): boolean {
  return useContext(PageScrolled)
}
