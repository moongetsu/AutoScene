import { create } from "zustand"
import { readClipboard } from "@/lib/os"
import { type TextField, type TextState, captureText } from "@/lib/text-edit"

export type OpenedTextMenu = {
  field: TextField
  at: { x: number; y: number }
  text: TextState
  /** The clipboard's text: `undefined` until it is read, `null` without any. */
  clipboard: string | null | undefined
}

type TextMenuStore = {
  opened: OpenedTextMenu | null
  open: (field: TextField, at: { x: number; y: number }) => void
  close: () => void
}

/** The one text menu: opened by the window's menu on a right-click in a field (`AppMenu`). */
export const useTextMenu = create<TextMenuStore>((set, get) => ({
  opened: null,
  open: (field, at) => {
    // A right-click on a field that isn't focused yet: it is now, as a
    // native field would be (undo and redo need it).
    if (document.activeElement !== field) field.focus({ preventScroll: true })
    set({ opened: { field, at, text: captureText(field), clipboard: undefined } })
    void readClipboard().then((clipboard) => {
      const now = get().opened
      if (now?.field === field) set({ opened: { ...now, clipboard } })
    })
  },
  close: () => set({ opened: null }),
}))
