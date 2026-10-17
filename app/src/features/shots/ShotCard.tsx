import { memo } from "react"
import { Play } from "@/components/icons"
import type { CharacterShot, MediaItem } from "@/bindings"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { NoFaceBadge } from "./NoFaceBadge"
import { useScrub } from "./scrub"
import { ShotFrames } from "./ShotFrames"
import { formatShotLength } from "./shots"

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
  const scrub = useScrub(shot)
  const fps = media?.fps
  const length = fps ? formatShotLength(((shot.end - shot.start) * fps.den) / fps.num) : ""
  const where = fps ? `${media?.label ? `${media.label} · ` : ""}${timecode(shot.start, fps, false)}` : ""

  return (
    <button
      type="button"
      onClick={() => onOpen(shot)}
      {...scrub.handlers}
      aria-label={`Shot${where ? ` at ${where}` : ""}${length ? `, ${length}` : ""}${shot.faceless ? ", without their face" : ""}`}
      title={where ? `${where} · click to watch` : undefined}
      className={cn(
        "group relative block aspect-video w-full overflow-hidden rounded-xl bg-inset text-left outline-none picture-edge",
        "transition-[transform,box-shadow] duration-[var(--dur-med)] ease-out",
        "hover:-translate-y-0.5 hover:shadow-lift focus-visible:ring-2 focus-visible:ring-accent-line",
      )}
    >
      <ShotFrames shot={shot} scrub={scrub} />
      {length && (
        <span className="timecode absolute right-2 bottom-2 rounded-full bg-black/50 px-2 text-[11px] leading-5 text-white backdrop-blur-md">
          {length}
        </span>
      )}
      <span className="absolute top-2 left-2 flex items-center gap-1.5">
        {shot.faceless && <NoFaceBadge />}
        {media?.label && (
          <span className="timecode rounded-full bg-black/50 px-2 text-[11px] leading-5 text-white/90 opacity-0 backdrop-blur-md transition-opacity group-hover:opacity-100">
            {media.label}
          </span>
        )}
      </span>
      <Play
        weight="fill"
        className="absolute top-2 right-2 size-4 text-white opacity-0 drop-shadow transition-opacity group-hover:opacity-90"
        aria-hidden
      />
    </button>
  )
})
