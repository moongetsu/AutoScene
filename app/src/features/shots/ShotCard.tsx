import { memo, useState } from "react"
import { Play } from "@/components/icons"
import type { CharacterShot, MediaItem } from "@/bindings"
import { assetUrl } from "@/lib/media"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { CARD_ASPECT, cellAt, formatShotLength, restingCell, spriteStyle } from "./shots"

/**
 * A shot as a card: its sprite — frames spread over the shot — scrubbed by
 * moving the pointer across, the character's face in a corner, its length
 * and where it is. Click opens the moment in the player.
 */
export const ShotCard = memo(function ShotCard({
  shot,
  media,
  onOpen,
}: {
  shot: CharacterShot
  media?: MediaItem
  onOpen: (shot: CharacterShot) => void
}) {
  const [hover, setHover] = useState<number | null>(null)
  // Width ÷ height of one cell, once the sprite has loaded (16:9 until then).
  const [aspect, setAspect] = useState(CARD_ASPECT)
  const cells = shot.sprite ? Math.max(1, shot.cells) : 0
  const cell = hover ?? restingCell(cells)
  const fps = media?.fps
  const length = fps ? formatShotLength(((shot.end - shot.start) * fps.den) / fps.num) : ""
  const where = fps ? `${media?.label ? `${media.label} · ` : ""}${timecode(shot.start, fps, false)}` : ""

  return (
    <button
      type="button"
      onClick={() => onOpen(shot)}
      onPointerMove={(e) => {
        if (cells === 0) return
        const r = e.currentTarget.getBoundingClientRect()
        setHover(cellAt(e.clientX - r.left, r.width, cells))
      }}
      onPointerLeave={() => setHover(null)}
      aria-label={`Shot${where ? ` at ${where}` : ""}${length ? `, ${length}` : ""}`}
      title={where ? `${where} · click to watch` : undefined}
      className={cn(
        "group relative block aspect-video w-full overflow-hidden rounded-xl bg-inset text-left outline-none picture-edge",
        "transition-[transform,box-shadow] duration-[var(--dur-med)] ease-out",
        "hover:-translate-y-0.5 hover:shadow-lift focus-visible:ring-2 focus-visible:ring-accent-line",
      )}
    >
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
      {length && (
        <span className="timecode absolute right-2 bottom-2 rounded-full bg-black/50 px-2 text-[11px] leading-5 text-white backdrop-blur-md">
          {length}
        </span>
      )}
      {media?.label && (
        <span className="timecode absolute top-2 left-2 rounded-full bg-black/50 px-2 text-[11px] leading-5 text-white/90 opacity-0 backdrop-blur-md transition-opacity group-hover:opacity-100">
          {media.label}
        </span>
      )}
      <Play
        weight="fill"
        className="absolute top-2 right-2 size-4 text-white opacity-0 drop-shadow transition-opacity group-hover:opacity-90"
        aria-hidden
      />
      {hover != null && cells > 1 && (
        <span
          className="pointer-events-none absolute bottom-0 left-0 h-[3px] bg-accent shadow-[0_0_8px_var(--accent)]"
          style={{ width: `${((hover + 1) * 100) / cells}%` }}
        />
      )}
    </button>
  )
})
