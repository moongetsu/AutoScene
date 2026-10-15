import { useState } from "react"
import type { CharacterShot } from "@/bindings"
import { assetUrl } from "@/lib/media"
import type { Scrub } from "./scrub"
import { CARD_ASPECT, spriteStyle } from "./shots"

/**
 * A shot's picture inside a 16:9 card: its sprite — frames spread over the
 * shot — at the scrubbed cell, the character's face in a corner, and how far
 * along the pointer is. Without a sprite, the face fills the card.
 */
export function ShotFrames({ shot, scrub }: { shot: CharacterShot; scrub: Scrub }) {
  // Width ÷ height of one cell, once the sprite has loaded (16:9 until then).
  const [aspect, setAspect] = useState(CARD_ASPECT)
  const { cells, cell, hover } = scrub
  return (
    <>
      {shot.sprite ? (
        // One image for every cell: moved, never reloaded.
        <img
          src={assetUrl(shot.sprite)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={(e) => {
            const img = e.currentTarget
            if (img.naturalHeight > 0) setAspect(img.naturalWidth / cells / img.naturalHeight)
          }}
          className="absolute top-1/2 left-1/2 max-w-none select-none"
          style={spriteStyle(cells, cell, aspect)}
        />
      ) : (
        <img
          src={assetUrl(shot.face)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className="absolute inset-0 size-full scale-110 object-cover opacity-60 blur-[1px]"
        />
      )}
      <span className="pointer-events-none absolute inset-x-0 bottom-0 h-14 bg-gradient-to-t from-black/70 to-transparent" />
      {shot.sprite && (
        <img
          src={assetUrl(shot.face)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className="absolute bottom-2 left-2 size-7 rounded-full object-cover shadow-1 ring-2 ring-black/40"
        />
      )}
      {hover != null && cells > 1 && (
        <span
          className="pointer-events-none absolute bottom-0 left-0 h-[3px] bg-accent shadow-[0_0_8px_var(--accent)]"
          style={{ width: `${((hover + 1) * 100) / cells}%` }}
        />
      )}
    </>
  )
}
