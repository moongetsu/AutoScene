import { memo } from "react"
import { Check, Play } from "@/components/icons"
import type { MediaItem, PackClip } from "@/bindings"
import { ShotFrames } from "@/features/shots/ShotFrames"
import { useScrub } from "@/features/shots/scrub"
import { formatShotLength } from "@/features/shots/shots"
import { matchShortcut, PACK_SHORTCUTS } from "@/lib/shortcuts"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { REASONS, SIZES, pickable } from "./spec"

const SIZE_SHORT = new Map(SIZES.map((s) => [s.value, s.short]))
const SIZE_LABEL = new Map(SIZES.map((s) => [s.value, s.label]))

/**
 * A shot of the pack: bright with a gold tick when the pack takes it,
 * dimmed with the reason when it doesn't. A click (or Space) takes it in
 * or out, with Shift every shot from the last one clicked; Enter or the
 * corner button plays it.
 */
export const PackCard = memo(function PackCard({
  clip,
  index,
  media,
  onToggle,
  onOpen,
}: {
  clip: PackClip
  index: number
  media?: MediaItem
  onToggle: (index: number, range: boolean) => void
  onOpen: (index: number) => void
}) {
  const scrub = useScrub(clip.shot)
  const fps = media?.fps
  const length = fps ? formatShotLength(((clip.end - clip.start) * fps.den) / fps.num) : ""
  const where = fps ? `${media?.label ? `${media.label} · ` : ""}${timecode(clip.start, fps, false)}` : ""
  const why = clip.included ? null : clip.reason ? REASONS[clip.reason] : "Left out"
  const canPick = pickable(clip)

  return (
    <div className="group relative aspect-video w-full">
      <button
        type="button"
        aria-pressed={clip.included}
        disabled={!canPick}
        onClick={(e) => onToggle(index, e.shiftKey)}
        onKeyDown={(e) => {
          const id = matchShortcut(PACK_SHORTCUTS, e.nativeEvent)
          if (id === "toggle") {
            e.preventDefault()
            onToggle(index, e.shiftKey)
          } else if (id === "watch") {
            e.preventDefault()
            onOpen(index)
          }
        }}
        {...scrub.handlers}
        aria-label={`Shot${where ? ` at ${where}` : ""}${length ? `, ${length}` : ""}${clip.included ? ", in the pack" : `, left out: ${why}`}`}
        title={where || undefined}
        className={cn(
          "relative block size-full overflow-hidden rounded-xl bg-inset text-left outline-none picture-edge",
          "transition-[transform,box-shadow] duration-[var(--dur-med)] ease-out focus-visible:ring-2 focus-visible:ring-accent-line",
          canPick && "hover:-translate-y-0.5 hover:shadow-lift",
          clip.included && "ring-1 ring-accent-line/70",
        )}
      >
        <span
          className={cn(
            "absolute inset-0 transition-[opacity,filter] duration-[var(--dur-med)]",
            !clip.included && "opacity-40 grayscale-[70%] group-hover:opacity-70",
          )}
        >
          <ShotFrames shot={clip.shot} scrub={scrub} />
        </span>
        <span className="absolute top-2 left-2 flex items-center gap-1.5">
          {clip.included ? (
            <span className="flex size-5 items-center justify-center rounded-full bg-accent text-on-accent shadow-1">
              <Check weight="bold" className="size-3" />
            </span>
          ) : (
            <span className="rounded-full bg-black/65 px-2 text-[11px] leading-5 text-white/90 backdrop-blur-md">{why}</span>
          )}
          {clip.manual === true && (
            <span className="rounded-full bg-black/55 px-2 text-[11px] leading-5 text-white/80 backdrop-blur-md">Added</span>
          )}
        </span>
        <span className="absolute right-2 bottom-2 flex items-center gap-1.5">
          <span
            title={SIZE_LABEL.get(clip.size)}
            className="rounded-full bg-black/50 px-2 text-[11px] leading-5 font-medium text-white/80 backdrop-blur-md"
          >
            {SIZE_SHORT.get(clip.size)}
          </span>
          {length && (
            <span className="timecode rounded-full bg-black/50 px-2 text-[11px] leading-5 text-white backdrop-blur-md">{length}</span>
          )}
        </span>
      </button>
      <button
        type="button"
        onClick={() => onOpen(index)}
        aria-label={`Watch the shot${where ? ` at ${where}` : ""}`}
        className={cn(
          "absolute top-2 right-2 flex size-7 items-center justify-center rounded-full bg-black/55 text-white backdrop-blur-md",
          "opacity-0 transition-opacity group-hover:opacity-100 hover:bg-black/75 focus-visible:opacity-100",
        )}
      >
        <Play weight="fill" className="size-3.5" />
      </button>
    </div>
  )
})
