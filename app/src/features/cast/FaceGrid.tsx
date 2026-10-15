import { memo, useRef } from "react"
import { Check } from "@/components/icons"
import type { FaceTrackItem, MediaItem } from "@/bindings"
import { assetUrl } from "@/lib/media"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"

/**
 * Face tracks as a grid of thumbnails. Click selects (Shift extends the
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

  const click = (i: number, e: React.MouseEvent) => {
    const id = tracks[i].id
    const next = new Set(selected)
    if (e.shiftKey && anchor.current != null) {
      const a = tracks.findIndex((t) => t.id === anchor.current)
      const [lo, hi] = a < 0 ? [i, i] : [Math.min(a, i), Math.max(a, i)]
      for (let k = lo; k <= hi; k++) next.add(tracks[k].id)
    } else if (e.ctrlKey || e.metaKey || selected.size > 0) {
      if (next.has(id)) next.delete(id)
      else next.add(id)
      anchor.current = id
    } else {
      next.add(id)
      anchor.current = id
    }
    onSelect(next)
  }

  return (
    <div className="grid gap-3" style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${size}px, 1fr))` }}>
      {tracks.map((t, i) => (
        <Face
          key={t.id}
          track={t}
          media={media.get(t.mediaId)}
          selected={selected.has(t.id)}
          onClick={(e) => click(i, e)}
          onOpen={() => onOpen(t)}
        />
      ))}
    </div>
  )
}

const Face = memo(function Face({
  track: t,
  media: m,
  selected,
  onClick,
  onOpen,
}: {
  track: FaceTrackItem
  media?: MediaItem
  selected: boolean
  onClick: (e: React.MouseEvent) => void
  onOpen: () => void
}) {
  const where = m?.fps ? `${m.label ? `${m.label} · ` : ""}${timecode(t.start, m.fps, false)}` : ""
  return (
    <button
      type="button"
      onClick={onClick}
      onDoubleClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") onOpen()
      }}
      aria-pressed={selected}
      aria-label={`Face${where ? ` at ${where}` : ""}`}
      title={where ? `${where} · double-click to watch` : undefined}
      className={cn(
        "group relative aspect-square overflow-hidden rounded-xl bg-inset outline-none picture-edge",
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
  )
})
