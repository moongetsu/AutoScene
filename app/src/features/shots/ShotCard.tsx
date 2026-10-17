import { memo } from "react"
import { Copy, Play, Prohibit, UserCircleDashed } from "@/components/icons"
import type { CharacterShot, MediaItem } from "@/bindings"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { Tooltip } from "@/components/ui/tooltip"
import { copyText } from "@/lib/os"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { NoFaceBadge } from "./NoFaceBadge"
import { useScrub } from "./scrub"
import { ShotFrames } from "./ShotFrames"
import { type BodyVerdict, formatShotLength } from "./shots"

/**
 * A shot as a card: its sprite — frames spread over the shot — scrubbed by
 * moving the pointer across, the character's face in a corner, its length
 * and where it is. Click opens the moment in the player.
 */
export const ShotCard = memo(function ShotCard({
  shot,
  media,
  name,
  onOpen,
  onBody,
}: {
  shot: CharacterShot
  media?: MediaItem
  /** Whose shot it is. */
  name?: string
  onOpen: (shot: CharacterShot) => void
  onBody?: (bodies: number[], verdict: BodyVerdict) => void
}) {
  const scrub = useScrub(shot)
  const fps = media?.fps
  const length = fps ? formatShotLength(((shot.end - shot.start) * fps.den) / fps.num) : ""
  const where = fps ? `${media?.label ? `${media.label} · ` : ""}${timecode(shot.start, fps, false)}` : ""
  // Drop-frame where the rate counts it, like the files' names.
  const at = fps ? timecode(shot.start, fps, true) : ""
  const bodies = shot.faceless ? (shot.bodies ?? []) : []
  const menu = (
    <>
      <ContextMenuItem onSelect={() => onOpen(shot)}>
        <Play /> Watch shot
      </ContextMenuItem>
      {at && (
        <ContextMenuItem onSelect={() => void copyText(at, "timecode")}>
          <Copy /> Copy timecode
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      {bodies.length > 0 && onBody && (
        <>
          <ContextMenuItem onSelect={() => onBody(bodies, "not_them")}>
            <UserCircleDashed /> Not {name ?? "them"}
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => onBody(bodies, "not_a_person")}>
            <Prohibit /> Not a person
          </ContextMenuItem>
        </>
      )}
    </>
  )

  return (
    <ContextArea menu={menu}>
      <Tooltip
        label={where}
        hint={shot.faceless ? "Their face doesn't show: found by their body · Click to watch" : "Click to watch"}
        side="top"
        delay={600}
        disabled={!where}
      >
        <button
          type="button"
          onClick={() => onOpen(shot)}
          {...scrub.handlers}
          aria-label={`Shot${where ? ` at ${where}` : ""}${length ? `, ${length}` : ""}${shot.faceless ? ", without their face" : ""}`}
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
      </Tooltip>
    </ContextArea>
  )
})
