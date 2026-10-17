/**
 * What the text fields' menu (`components/shell/TextMenu`) does to a field:
 * the editing a native field offers — undo, redo, cut, copy, paste, delete,
 * select all — done the way typing does it, so it can be undone and React's
 * `onChange` hears it. Copy and paste go through the app's own clipboard
 * commands (`lib/os`), never the webview's (which would ask for permission).
 */
import { isMac } from "./platform"

export type TextField = HTMLInputElement | HTMLTextAreaElement

export type TextAction = "undo" | "redo" | "cut" | "copy" | "paste" | "delete" | "selectAll"

/** Inputs that hold text one edits (a number field has no selection to edit). */
const TEXT_TYPES = new Set(["text", "search", "url", "email", "tel", "password"])

/** The text field a right-click landed in, if any. */
export function textFieldAt(target: EventTarget | null): TextField | null {
  if (!(target instanceof Element)) return null
  const el = target.closest("input, textarea")
  if (el instanceof HTMLTextAreaElement) return el
  if (el instanceof HTMLInputElement && TEXT_TYPES.has(el.type)) return el
  return null
}

/** The field as the menu opened on it: what is selected, what can be done. */
export type TextState = {
  start: number
  end: number
  direction: "forward" | "backward" | "none"
  selected: string
  readOnly: boolean
  /** A password: what is in it is never copied out. */
  secret: boolean
  empty: boolean
  /** All of it is selected already. */
  all: boolean
  canUndo: boolean
  canRedo: boolean
}

function query(command: string): boolean {
  try {
    return document.queryCommandEnabled(command)
  } catch {
    return false
  }
}

/** Read `field` as it is now (it must have the focus, for undo and redo). */
export function captureText(field: TextField): TextState {
  const start = field.selectionStart ?? 0
  const end = field.selectionEnd ?? start
  const value = field.value
  return {
    start,
    end,
    direction: field.selectionDirection ?? "none",
    selected: value.slice(start, end),
    readOnly: field.readOnly || field.disabled,
    secret: field instanceof HTMLInputElement && field.type === "password",
    empty: value.length === 0,
    all: value.length > 0 && start === 0 && end === value.length,
    canUndo: query("undo"),
    canRedo: query("redo"),
  }
}

/** Whether `action` can be done; Paste needs to know if the clipboard holds text (`null`: not known yet). */
export function canDo(state: TextState, action: TextAction, clipboard: string | null | undefined): boolean {
  const some = state.selected.length > 0
  switch (action) {
    case "undo":
      return !state.readOnly && state.canUndo
    case "redo":
      return !state.readOnly && state.canRedo
    case "cut":
      return !state.readOnly && !state.secret && some
    case "copy":
      return !state.secret && some
    case "paste":
      return !state.readOnly && typeof clipboard === "string" && clipboard.length > 0
    case "delete":
      return !state.readOnly && some
    case "selectAll":
      return !state.empty && !state.all
  }
}

/** The keys that do the same, as a menu shows them. */
export function textShortcut(action: TextAction): string[] | undefined {
  const mod = isMac ? "⌘" : "Ctrl"
  switch (action) {
    case "undo":
      return [mod, "Z"]
    case "redo":
      return isMac ? ["⇧", "⌘", "Z"] : ["Ctrl", "Y"]
    case "cut":
      return [mod, "X"]
    case "copy":
      return [mod, "C"]
    case "paste":
      return [mod, "V"]
    case "delete":
      return [isMac ? "⌫" : "Del"]
    case "selectAll":
      return [mod, "A"]
  }
}

function exec(command: string, value?: string): boolean {
  try {
    return typeof document.execCommand === "function" && document.execCommand(command, false, value)
  } catch {
    return false
  }
}

/** The field focused again, with the selection the menu was opened on. */
export function restoreSelection(field: TextField, state: TextState) {
  if (document.activeElement !== field) field.focus({ preventScroll: true })
  field.setSelectionRange(state.start, state.end, state.direction)
}

/**
 * Put `text` where the selection is, as typing would: it can be undone and
 * React hears an input event. Without `execCommand` (tests, other engines)
 * the value is changed directly — React still hears it.
 */
export function insertText(field: TextField, text: string) {
  if (exec("insertText", text)) return
  const start = field.selectionStart ?? field.value.length
  const end = field.selectionEnd ?? start
  field.setRangeText(text, start, end, "end")
  field.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }))
}

/** Remove the selection, as the Delete key would. */
export function deleteSelection(field: TextField) {
  const start = field.selectionStart ?? 0
  const end = field.selectionEnd ?? start
  if (start === end) return
  if (exec("delete")) return
  field.setRangeText("", start, end, "end")
  field.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward" }))
}

/** A one-line field takes pasted lines as one line. */
export function pastable(field: TextField, text: string): string {
  return field instanceof HTMLInputElement ? text.replace(/\r\n|\r|\n/g, " ") : text
}

/**
 * Do `action` on `field`, as the menu found it (`state`). `copy` writes the
 * clipboard (`lib/os` copyText) and tells whether it worked; `clipboard` is
 * what Paste puts in.
 */
export async function runTextAction(
  field: TextField,
  state: TextState,
  action: TextAction,
  { copy, clipboard }: { copy: (text: string) => Promise<boolean>; clipboard: string | null | undefined },
): Promise<void> {
  restoreSelection(field, state)
  switch (action) {
    case "undo":
      exec("undo")
      return
    case "redo":
      exec("redo")
      return
    case "copy":
      await copy(state.selected)
      return
    case "cut":
      if (await copy(state.selected)) {
        restoreSelection(field, state)
        deleteSelection(field)
      }
      return
    case "delete":
      deleteSelection(field)
      return
    case "paste":
      if (clipboard) insertText(field, pastable(field, clipboard))
      return
    case "selectAll":
      field.select()
      return
  }
}
