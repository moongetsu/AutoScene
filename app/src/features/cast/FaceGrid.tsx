import { memo, useRef } from "react"
import { Check } from "lucide-react"
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
  size = 96,
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
    <div className="grid gap-2" style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${size}px, 1fr))` }}>
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
        "group relative aspect-square overflow-hidden rounded-md bg-inset outline-none",
        "ring-offset-2 ring-offset-canvas transition-[box-shadow,opacity] duration-[var(--dur-fast)]",
        "focus-visible:ring-2 focus-visible:ring-accent-line",
        selected ? "ring-2 ring-accent" : "hover:ring-1 hover:ring-strong",
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
        <span className="absolute top-1 left-1 size-1.5 rounded-full bg-fg-1/80 shadow-1" aria-label="Confirmed" />
      )}
      <span
        className={cn(
          "absolute top-1 right-1 flex size-4 items-center justify-center rounded-full border transition-opacity",
          selected
            ? "border-accent bg-accent text-on-accent opacity-100"
            : "border-fg-1/60 bg-canvas/40 opacity-0 group-hover:opacity-100",
        )}
      >
        {selected && <Check className="size-3" strokeWidth={3} />}
      </span>
      {m?.label && (
        <span className="timecode absolute right-1 bottom-1 rounded-xs bg-canvas/70 px-1 text-[10px] leading-4 text-fg-2 opacity-0 transition-opacity group-hover:opacity-100">
          {m.label}
        </span>
      )}
    </button>
  )
})
