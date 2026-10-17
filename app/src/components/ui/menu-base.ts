import { cn } from "@/lib/utils"

/** The surface every menu of the app shares (the text fields' too). */
export const menuSurface =
  "glass-strong z-50 min-w-[200px] rounded-xl border border-white/10 p-1.5 shadow-2xl focus:outline-none"

/** One entry of a menu. */
export const menuItem = cn(
  "flex h-9 cursor-default items-center gap-3 rounded-lg px-3 text-md text-fg-2 outline-none select-none",
  "data-[highlighted]:bg-active data-[highlighted]:text-fg-1 data-[disabled]:opacity-40",
  "[&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-3 data-[highlighted]:[&_svg]:text-fg-2",
)

export const menuSeparator = "mx-2 my-1.5 h-px bg-hairline"

/**
 * An entry's action, run once its menu has closed. An open menu holds the
 * focus — it takes it back from a field the action focuses (the go-to field,
 * a name to edit) — and Radix runs the action while it is still open. Later,
 * the field, a dialog or another menu keeps the focus it is given.
 */
export function afterClose<E>(action: ((e: E) => void) | undefined): ((e: E) => void) | undefined {
  return action && ((e: E) => void setTimeout(() => action(e), 0))
}
