/**
 * The few things the app asks of the system: copying text (natively — the
 * webview's clipboard API needs a user gesture or a permission prompt),
 * reading the clipboard's text for Paste, and showing a file or folder in
 * Explorer / Finder. Failures say so in a toast.
 */
import { toast } from "sonner"
import { commands, errorMessage } from "./ipc"

/**
 * Copy `text`. With `what` ("timecode", "file path") a toast says what was
 * copied; without it the copy is quiet, like a text field's.
 */
export async function copyText(text: string, what?: string): Promise<boolean> {
  try {
    await commands.clipboardWrite(text)
    if (what) toast(`Copied ${what}`, { description: text.length > 96 ? `${text.slice(0, 93)}…` : text })
    return true
  } catch (e) {
    toast.error(what ? `Couldn't copy the ${what}` : "Couldn't copy", { description: errorMessage(e) })
    return false
  }
}

/** The clipboard's text, or null (a picture, files, nothing, no access). */
export async function readClipboard(): Promise<string | null> {
  try {
    return await commands.clipboardRead()
  } catch {
    return null
  }
}

/** Show a file or folder in Explorer / Finder; a toast when it isn't there. */
export function reveal(path: string): void {
  commands.revealPath(path).catch((e) => toast.error("Couldn't show it", { description: errorMessage(e) }))
}
