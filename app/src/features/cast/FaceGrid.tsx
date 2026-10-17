import { memo, useCallback, useLayoutEffect, useRef } from "react"
import { Check, Play } from "@/components/icons"
import type { FaceTrackItem, MediaItem } from "@/bindings"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { Tooltip } from "@/components/ui/tooltip"
import { VirtualGrid } from "@/components/ui/virtual-grid"
import { clickSelect } from "@/lib/grid"
import { assetUrl } from "@/lib/media"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"

/**
 * Face tracks as a grid of thumbnails (only the rows on screen are drawn:
 * "Everyone else" can hold thousands). Click selects (Shift extends the
 * range, Ctrl/⌘ toggles one), double-click opens the moment in the player.
 */
export function FaceGrid({
  tracks,
  media,
  selected,
  onSelect,
  onOpen,
  size = 108,
}: {
  tracks: FaceTrackItem[]
  media: Map<string, MediaItem>
  selected: Set<number>
  onSelect: (next: Set<number>) => void
  onOpen: (t: FaceTrackItem) => void
  size?: number
}) {
  const anchor = useRef<number | null>(null)
  // The faces call back with their index; the latest list and selection
  // answer, so a face only redraws when its own picture or state changes.
  const latest = useRef({ tracks, selected, onSelect, onOpen })
  useLayoutEffect(() => {
    latest.current = { tracks, selected, onSelect, onOpen }
  })
  const pick = useCallback((index: number, e: React.MouseEvent) => {
    const { tracks, selected, onSelect } = latest.current
    const ids = tracks.map((t) => t.id)
    const next = clickSelect(ids, selected, anchor.current, index, { shift: e.shiftKey, toggle: e.ctrlKey || e.metaKey })
    anchor.current = next.anchor
    onSelect(next.selected)
  }, [])
  const open = useCallback((index: number) => {
    const t = latest.current.tracks[index]
    if (t) latest.current.onOpen(t)
  }, [])
  // A right-click on a face not selected selects it alone, as files do: the
  // menu's selection entries then speak of it.
  const contextPick = useCallback((index: number) => {
    const { tracks, selected, onSelect } = latest.current
    const t = tracks[index]
    if (!t || selected.has(t.id)) return
    anchor.current = index
    onSelect(new Set([t.id]))
  }, [])

  return (
    <VirtualGrid
      items={tracks}
      minItemWidth={size}
      itemHeight={(w) => w}
      gap={12}
      getKey={(t) => t.id}
      renderItem={(t, index) => (
        <Face
          track={t}
          index={index}
          media={media.get(t.mediaId)}
          selected={selected.has(t.id)}
          onPick={pick}
          onOpen={open}
          onContextPick={contextPick}
          onWatch={onOpen}
        />
      )}
    />
  )
}

const Face = memo(function Face({
  track: t,
  index,
  media: m,
  selected,
  onPick,
  onOpen,
  onContextPick,
  onWatch,
}: {
  track: FaceTrackItem
  index: number
  media?: MediaItem
  selected: boolean
  onPick: (index: number, e: React.MouseEvent) => void
  onOpen: (index: number) => void
  onContextPick: (index: number) => void
  /** Watch this face's moment (the face itself, not its place in the grid). */
  onWatch: (t: FaceTrackItem) => void
}) {
  const where = m?.fps ? `${m.label ? `${m.label} · ` : ""}${timecode(t.start, m.fps, false)}` : ""
  return (
    <Tooltip label={where} hint="Click to select · Double-click to watch" side="top" delay={600} disabled={!where}>
      <ContextArea
        menu={
          <ContextMenuItem onSelect={() => onWatch(t)}>
            <Play /> Watch this moment
          </ContextMenuItem>
        }
      >
        <button
          type="button"
          onContextMenu={() => onContextPick(index)}
          onClick={(e) => onPick(index, e)}
          onDoubleClick={() => onOpen(index)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onOpen(index)
          }}
          aria-pressed={selected}
          aria-label={`Face${where ? ` at ${where}` : ""}`}
          className={cn(
            "group relative block aspect-square w-full overflow-hidden rounded-xl bg-inset outline-none picture-edge",
            "ring-offset-2 ring-offset-canvas transition-[box-shadow,opacity,transform] duration-[var(--dur-fast)]",
            "focus-visible:ring-2 focus-visible:ring-accent-line",
            selected ? "scale-[0.96] ring-2 ring-accent" : "hover:ring-1 hover:ring-white/30",
          )}
        >
          <img
            src={assetUrl(t.thumb)}
            alt=""
            loading="lazy"
            decoding="async"
            draggable={false}
            className={cn("size-full object-cover", (t.quality ?? 0) < 0.3 && "opacity-70")}
          />
          {t.assign === "pinned" && !selected && (
            <span className="absolute top-2 left-2 size-2 rounded-full bg-white shadow-[0_0_6px_rgb(255_255_255/0.6)]" aria-label="Confirmed" />
          )}
          <span
            className={cn(
              "absolute top-1.5 right-1.5 flex size-5 items-center justify-center rounded-full border-2 transition-opacity",
              selected
                ? "border-accent bg-accent text-on-accent opacity-100"
                : "border-white/70 bg-black/30 opacity-0 backdrop-blur-sm group-hover:opacity-100",
            )}
          >
            {selected && <Check className="size-3" weight="bold" />}
          </span>
          {m?.label && (
            <span className="timecode absolute right-1.5 bottom-1.5 rounded-full bg-black/55 px-1.5 text-[10px] leading-4 text-white/90 opacity-0 backdrop-blur-md transition-opacity group-hover:opacity-100">
              {m.label}
            </span>
          )}
        </button>
      </ContextArea>
    </Tooltip>
  )
})
