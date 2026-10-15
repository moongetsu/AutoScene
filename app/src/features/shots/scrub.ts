import type * as React from "react"
import { useState } from "react"
import type { CharacterShot } from "@/bindings"
import { cellAt, restingCell } from "./shots"

export type Scrub = ReturnType<typeof useScrub>

/** Moving the pointer across a card scrubs its shot: the cell to show, and the handlers for the card. */
export function useScrub(shot: CharacterShot) {
  const [hover, setHover] = useState<number | null>(null)
  const cells = shot.sprite ? Math.max(1, shot.cells) : 0
  return {
    cells,
    hover,
    cell: hover ?? restingCell(cells),
    handlers: {
      onPointerMove: (e: React.PointerEvent<HTMLElement>) => {
        if (cells === 0) return
        const r = e.currentTarget.getBoundingClientRect()
        setHover(cellAt(e.clientX - r.left, r.width, cells))
      },
      onPointerLeave: () => setHover(null),
    },
  }
}
