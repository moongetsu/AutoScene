import { Link } from "@tanstack/react-router"
import { User, UserFocus } from "@/components/icons"
import type { Character } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"
import { formatScreenTime, plural } from "./cast"

/** A character as a portrait: the face fills the card, the name sits on it. */
export function CharacterCard({ pid, character: c, className }: { pid: string; character: Character; className?: string }) {
  // Their episode is being analysed again: the faces come back when it's done.
  const refinding = c.tracks === 0 && c.waiting > 0
  return (
    <Link
      to="/p/$pid/cast/$cid"
      params={{ pid, cid: c.id }}
      className={cn(
        "group block rounded-2xl transition-transform duration-[var(--dur-med)] ease-out hover:-translate-y-1 focus-visible:-translate-y-1",
        refinding && "opacity-60",
        className,
      )}
    >
      <Artwork
        path={c.cover}
        className="aspect-[3/4] rounded-2xl picture-edge transition-shadow duration-[var(--dur-med)] group-hover:shadow-lift"
        imgClassName="group-hover:scale-[1.05] duration-[600ms]"
        fallback={
          <div className="flex size-full items-center justify-center artwork-fallback text-fg-3 [&_svg]:size-12">
            {refinding ? <UserFocus /> : <User />}
          </div>
        }
      >
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/90 via-black/15 to-transparent" />
        {c.unsure > 0 && (
          <Badge tone="media" className="absolute top-2.5 right-2.5">
            +{c.unsure} to review
          </Badge>
        )}
        <div className="absolute inset-x-0 bottom-0 p-3.5">
          <div className={cn("display truncate text-2xl leading-7", c.name ? "text-white" : "text-white/70 italic")}>{c.label}</div>
          <div className="mt-0.5 truncate text-xs text-white/70">
            {refinding ? (
              "Being found again…"
            ) : (
              <>
                {formatScreenTime(c.screenSec ?? 0)} · {plural(c.shots, "shot")}
              </>
            )}
          </div>
        </div>
      </Artwork>
    </Link>
  )
}
