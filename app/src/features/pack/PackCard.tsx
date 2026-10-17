import { memo } from "react"
import { Check, Copy, Play, Prohibit, UserCircleDashed } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem, PackClip } from "@/bindings"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { NoFaceBadge } from "@/features/shots/NoFaceBadge"
import { ShotFrames } from "@/features/shots/ShotFrames"
import { useScrub } from "@/features/shots/scrub"
import { formatShotLength } from "@/features/shots/shots"
import { commands, errorMessage } from "@/lib/ipc"
import { matchShortcut, PACK_SHORTCUTS } from "@/lib/shortcuts"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { REASONS, SIZES, pickable } from "./spec"

const SIZE_SHORT = new Map(SIZES.map((s) => [s.value, s.short]))
const SIZE_LABEL = new Map(SIZES.map((s) => [s.value, s.label]))

/** What can be said about the bodies a shot without the character's face shows. */
export type BodyVerdict = "not_them" | "not_a_person"

/**
 * A shot of the pack: bright with a gold tick when the pack takes it,
 * dimmed with the reason when it doesn't. A click (or Space) takes it in
 * or out, with Shift every shot from the last one clicked; Enter or the
 * corner button plays it. A shot without the character's face says so;
 * its menu can say the bodies there aren't them.
 */
export const PackCard = memo(function PackCard({
  clip,
  index,
  media,
  name,
  onToggle,
  onOpen,
  onBody,
}: {
  clip: PackClip
  index: number
  media?: MediaItem
  /** Whose pack it is. */
  name?: string
  onToggle: (index: number, range: boolean, byKey?: boolean) => void
  onOpen: (index: number) => void
  onBody?: (bodies: number[], verdict: BodyVerdict) => void
}) {
  const scrub = useScrub(clip.shot)
  const fps = media?.fps
  // The shot itself, as the filters judge it (handles only widen the clip).
  const { start, end } = clip.shot
  const length = fps ? formatShotLength(((end - start) * fps.den) / fps.num) : ""
  // Drop-frame where the rate counts it, like the files' names.
  const at = fps ? timecode(start, fps, true) : ""
  const where = at && `${media?.label ? `${media.label} · ` : ""}${at}`
  const why = clip.included ? null : clip.reason ? REASONS[clip.reason] : "Left out"
  const canPick = pickable(clip)
  const bodies = clip.shot.faceless ? (clip.shot.bodies ?? []) : []

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="group relative aspect-video w-full">
          <button
            type="button"
            aria-pressed={clip.included}
            disabled={!canPick}
            onClick={(e) => onToggle(index, e.shiftKey)}
            data-pack-index={index}
            onKeyDown={(e) => {
              const id = matchShortcut(PACK_SHORTCUTS, e.nativeEvent)
              if (id === "toggle") {
                e.preventDefault()
                // A held key would take the shot in and out again and again.
                if (!e.repeat) onToggle(index, e.shiftKey, true)
              } else if (id === "watch") {
                e.preventDefault()
                onOpen(index)
              }
            }}
            {...scrub.handlers}
            aria-label={`Shot${where ? ` at ${where}` : ""}${length ? `, ${length}` : ""}${clip.shot.faceless ? ", without their face" : ""}${clip.included ? ", in the pack" : `, left out: ${why}`}`}
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
              {clip.shot.faceless && clip.reason !== "faceless" && <NoFaceBadge />}
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
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => onOpen(index)} shortcut={["Enter"]}>
          <Play /> Watch shot
        </ContextMenuItem>
        {canPick && (
          <ContextMenuItem onSelect={() => onToggle(index, false)} shortcut={["Space"]}>
            <Check /> {clip.included ? "Remove from pack" : "Add to pack"}
          </ContextMenuItem>
        )}
        {bodies.length > 0 && onBody && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem onSelect={() => onBody(bodies, "not_them")}>
              <UserCircleDashed /> Not {name ?? "them"}
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => onBody(bodies, "not_a_person")}>
              <Prohibit /> Not a person
            </ContextMenuItem>
          </>
        )}
        {where && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem
              onSelect={() =>
                commands
                  .clipboardWrite(at)
                  .then(() => toast("Copied timecode", { description: where }))
                  .catch((e) => toast.error("Couldn't copy the timecode", { description: errorMessage(e) }))
              }
            >
              <Copy /> Copy timecode
            </ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  )
})
