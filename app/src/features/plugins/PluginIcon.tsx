import { PuzzlePiece } from "@/components/icons"
import { cn } from "@/lib/utils"

/** What a plugin's icon may be: its PNG or SVG as a base64 `data:` URL (the backend reads and checks the file). */
const DATA_URL = /^data:image\/(?:png|svg\+xml);base64,[A-Za-z0-9+/]+={0,2}$/

/**
 * A plugin's icon, else the puzzle piece. Only ever a picture: an SVG in an
 * `<img>` runs no script and loads nothing, so it is never drawn inline.
 */
export function PluginIcon({ src, className }: { src: string | null | undefined; className?: string }) {
  if (src != null && DATA_URL.test(src)) {
    return <img src={src} alt="" draggable={false} className={cn("size-4 shrink-0 object-contain", className)} />
  }
  return <PuzzlePiece className={className} />
}
