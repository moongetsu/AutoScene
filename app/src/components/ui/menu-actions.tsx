import type * as React from "react"
import { ContextMenuItem, ContextMenuSeparator } from "./context-menu"
import { DropdownMenuItem, DropdownMenuSeparator } from "./dropdown-menu"

type ItemProps = {
  onSelect?: () => void
  destructive?: boolean
  disabled?: boolean
  shortcut?: string[]
  children: React.ReactNode
}

/**
 * The pieces a list of actions is drawn with: an element's right-click menu
 * and its ⋯ menu list the same actions in the same order, each with its own
 * kind of item.
 */
export type MenuKit = {
  Item: (props: ItemProps) => React.ReactNode
  Separator: () => React.ReactNode
}

export const contextKit: MenuKit = {
  Item: ({ onSelect, ...props }) => <ContextMenuItem onSelect={() => onSelect?.()} {...props} />,
  Separator: ContextMenuSeparator,
}

export const dropdownKit: MenuKit = {
  Item: ({ onSelect, ...props }) => <DropdownMenuItem onSelect={() => onSelect?.()} {...props} />,
  Separator: DropdownMenuSeparator,
}
