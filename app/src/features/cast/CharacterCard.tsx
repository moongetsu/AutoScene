import type * as React from "react"
import { Link } from "@tanstack/react-router"
import { Check, CheckCircle, User, UserFocus } from "@/components/icons"
import type { Character } from "@/bindings"
import { Artwork } from "@/components/ui/artwork"
import { Badge } from "@/components/ui/badge"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { contextKit } from "@/components/ui/menu-actions"
import { cn } from "@/lib/utils"
import { formatScreenTime, plural } from "./cast"
import { CharacterItems } from "./character-actions"

/**
 * A character as a portrait: the face fills the card, the name sits on it.
 * Where cards can be selected (`onPick`), a selected one wears a gold ring
 * and tick, and its right-click offers what the selection can be told.
 */
export function CharacterCard({
  pid,
  character: c,
  className,
  onRename,
  onChoosePicture,
  selected = false,
  selecting = false,
  onPick,
  onSelect,
  selectionMenu,
}: {
  pid: string
  character: Character
  className?: string
  onRename?: () => void
  onChoosePicture?: () => void
  selected?: boolean
  /** Something is selected: a click adds or takes away this card too. */
  selecting?: boolean
  /** A click on the card; `true` when the selection took it (the character isn't opened). */
  onPick?: (e: React.MouseEvent) => boolean
  /** Add this card to the selection, or start one with it (its menu's Select). */
  onSelect?: () => void
  /** What a right-click on the card offers while it is selected. */
  selectionMenu?: React.ReactNode
}) {
  // Their episode is being analysed again: the faces come back when it's done.
  const refinding = c.tracks === 0 && c.waiting > 0
  const menu =
    selected && selectionMenu ? (
      selectionMenu
    ) : (
      <>
        <CharacterItems kit={contextKit} pid={pid} character={c} onRename={onRename} onChoosePicture={onChoosePicture} />
        {onSelect && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem onSelect={onSelect}>
              <CheckCircle /> Select
            </ContextMenuItem>
          </>
        )}
      </>
    )
  return (
    <ContextArea menu={menu}>
        <Link
          to="/p/$pid/cast/$cid"
          params={{ pid, cid: c.id }}
          // In the capture phase: the browser guards stop a Ctrl/⌘- or
          // Shift-click on a link before it gets here, and the link then skips
          // its onClick. A click the selection takes doesn't open the character.
          onClickCapture={(e) => {
            if (onPick?.(e)) e.preventDefault()
          }}
          className={cn(
            "group block rounded-2xl transition-[transform,box-shadow] duration-[var(--dur-med)] ease-out hover:-translate-y-1 focus-visible:-translate-y-1",
            selected && "ring-2 ring-accent ring-offset-2 ring-offset-canvas",
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
            {(selected || selecting) && (
              <span
                className={cn(
                  "absolute top-2.5 left-2.5 flex size-6 items-center justify-center rounded-full border-2 transition-opacity",
                  selected
                    ? "border-accent bg-accent text-on-accent"
                    : "border-white/70 bg-black/30 opacity-0 backdrop-blur-sm group-hover:opacity-100",
                )}
              >
                {selected && (
                  <>
                    <Check className="size-3.5" weight="bold" />
                    <span className="sr-only">Selected</span>
                  </>
                )}
              </span>
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
    </ContextArea>
  )
}

/** A compact row for list view: avatar thumbnail, name, screen time, shots. */
export function CharacterRow({
  pid,
  character: c,
  onRename,
  onChoosePicture,
  selected = false,
  onPick,
  onSelect,
  selectionMenu,
}: {
  pid: string
  character: Character
  onRename?: () => void
  onChoosePicture?: () => void
  selected?: boolean
  onPick?: (e: React.MouseEvent) => boolean
  onSelect?: () => void
  selectionMenu?: React.ReactNode
}) {
  const refinding = c.tracks === 0 && c.waiting > 0
  const menu =
    selected && selectionMenu ? (
      selectionMenu
    ) : (
      <>
        <CharacterItems kit={contextKit} pid={pid} character={c} onRename={onRename} onChoosePicture={onChoosePicture} />
        {onSelect && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem onSelect={onSelect}>
              <CheckCircle /> Select
            </ContextMenuItem>
          </>
        )}
      </>
    )

  return (
    <ContextArea menu={menu}>
      <Link
        to="/p/$pid/cast/$cid"
        params={{ pid, cid: c.id }}
        onClickCapture={(e) => {
          if (onPick?.(e)) e.preventDefault()
        }}
        className={cn(
          "group flex items-center justify-between gap-4 rounded-xl px-4 py-2.5 transition-colors hover:bg-white/[0.04]",
          selected && "bg-accent/10 ring-1 ring-accent",
          refinding && "opacity-60",
        )}
      >
        <div className="flex min-w-0 items-center gap-3.5">
          <Artwork
            path={c.coverSmall ?? c.cover}
            className="size-10 shrink-0 rounded-lg picture-edge"
            fallback={
              <div className="flex size-full items-center justify-center artwork-fallback text-fg-3 [&_svg]:size-5">
                <User />
              </div>
            }
          />
          <div className="min-w-0">
            <div className={cn("truncate text-md font-medium", c.name ? "text-fg-1" : "text-fg-2 italic")}>
              {c.label}
            </div>
            <div className="text-xs text-fg-3">
              {refinding ? "Being found again…" : `${formatScreenTime(c.screenSec ?? 0)} · ${plural(c.shots, "shot")}`}
            </div>
          </div>
        </div>
        {c.unsure > 0 && (
          <span className="flex items-center gap-1.5 rounded-full border border-accent-line bg-accent-subtle px-2.5 py-0.5 text-xs font-semibold text-accent">
            {c.unsure} to review
          </span>
        )}
      </Link>
    </ContextArea>
  )
}
