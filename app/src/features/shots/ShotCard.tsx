import { memo, useState } from "react"
import { Play } from "lucide-react"
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
        "group relative aspect-video overflow-hidden rounded-md bg-inset text-left outline-none",
        "ring-offset-2 ring-offset-canvas transition-shadow duration-[var(--dur-fast)]",
        "hover:ring-1 hover:ring-strong focus-visible:ring-2 focus-visible:ring-accent-line",
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
      <span className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-gradient-to-t from-canvas/80 to-transparent" />
      {shot.sprite && (
        <img
          src={assetUrl(shot.face)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className="absolute bottom-1.5 left-1.5 size-6 rounded-full object-cover shadow-1 ring-1 ring-canvas/60"
        />
      )}
      {length && (
        <span className="timecode absolute right-1.5 bottom-1.5 rounded-xs bg-canvas/70 px-1 text-[10px] leading-4 text-fg-1">
          {length}
        </span>
      )}
      {media?.label && (
        <span className="timecode absolute top-1.5 left-1.5 rounded-xs bg-canvas/70 px-1 text-[10px] leading-4 text-fg-2 opacity-0 transition-opacity group-hover:opacity-100">
          {media.label}
        </span>
      )}
      <Play
        className="absolute top-1.5 right-1.5 size-3.5 fill-fg-1 text-fg-1 opacity-0 drop-shadow transition-opacity group-hover:opacity-90"
        aria-hidden
      />
      {hover != null && cells > 1 && (
        <span
          className="pointer-events-none absolute bottom-0 left-0 h-0.5 bg-accent"
          style={{ width: `${((hover + 1) * 100) / cells}%` }}
        />
      )}
    </button>
  )
})
