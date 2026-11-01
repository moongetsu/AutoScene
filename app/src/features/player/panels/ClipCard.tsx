import { ArrowLineLeft, ArrowLineRight, Copy, Crosshair, Scissors, X } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { Kbd } from "@/components/ui/kbd"
import { Tooltip } from "@/components/ui/tooltip"
import { keysText, useShortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { Card } from "./Card"

/** The marked clip: in and out, its length, and Export. */
export function ClipCard({
  inFrame,
  outFrame,
  length,
  tc,
  canExport,
  onMarkIn,
  onMarkOut,
  onGo,
  onClear,
  onExport,
  onCopy,
}: {
  inFrame: number | null
  outFrame: number | null
  /** Frames from in to out (both included). */
  length: number
  tc: (f: number) => string
  canExport: boolean
  onMarkIn: () => void
  onMarkOut: () => void
  onGo: (f: number) => void
  onClear: () => void
  onExport: () => void
  /** Copy the clip's in and out. */
  onCopy: () => void
}) {
  const keys = useShortcutKeys()
  const shotKey = keys("player.markShot")
  const hasMarks = inFrame != null || outFrame != null
  const menu = (
    <>
      <ContextMenuItem onSelect={onMarkIn} shortcut={keys("player.mark", 0)}>
        <ArrowLineRight /> Mark in
      </ContextMenuItem>
      <ContextMenuItem onSelect={onMarkOut} shortcut={keys("player.mark", 1)}>
        <ArrowLineLeft /> Mark out
      </ContextMenuItem>
      {inFrame != null && (
        <ContextMenuItem onSelect={() => onGo(inFrame)} shortcut={keys("player.goToMark", 0)}>
          <Crosshair /> Go to the in point
        </ContextMenuItem>
      )}
      {outFrame != null && (
        <ContextMenuItem onSelect={() => onGo(outFrame)} shortcut={keys("player.goToMark", 1)}>
          <Crosshair /> Go to the out point
        </ContextMenuItem>
      )}
      {hasMarks && (
        <ContextMenuItem onSelect={onClear} shortcut={keys("player.clearMarks")}>
          <X /> Clear in and out
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem disabled={!canExport} onSelect={onExport} shortcut={keys("player.export")}>
        <Scissors /> Export clip…
      </ContextMenuItem>
      {hasMarks && (
        <ContextMenuItem onSelect={onCopy}>
          <Copy /> Copy in and out
        </ContextMenuItem>
      )}
    </>
  )
  return (
    <Card
      title="Clip"
      menu={menu}
      action={
        hasMarks && (
          <Tooltip label="Clear in and out" shortcut={keys("player.clearMarks")}>
            <button
              onClick={onClear}
              aria-label="Clear in and out"
              className="flex size-7 items-center justify-center rounded-full text-fg-3 hover:bg-white/[0.08] hover:text-fg-1"
            >
              <X className="size-3.5" weight="bold" />
            </button>
          </Tooltip>
        )
      }
    >
      <div className="grid grid-cols-2 gap-2">
        <MarkTile label="In" slot={0} value={inFrame} tc={tc} onMark={onMarkIn} onGo={onGo} />
        <MarkTile label="Out" slot={1} value={outFrame} tc={tc} onMark={onMarkOut} onGo={onGo} />
      </div>
      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-fg-3">Duration</dt>
        <dd className="timecode text-right text-fg-1">{hasMarks ? tc(length) : "–"}</dd>
        <dt className="text-fg-3">Frames</dt>
        <dd className="timecode text-right text-fg-1">{hasMarks ? length : "–"}</dd>
      </dl>
      <Button variant="primary" size="lg" className="mt-4 w-full" disabled={!canExport} onClick={onExport}>
        <Scissors weight="bold" /> Export clip
        <Kbd keys={keys("player.export")} className="ml-auto" />
      </Button>
      {!hasMarks && (
        <p className="mt-2.5 text-center text-xs text-fg-3">
          Mark an in or out point, or a whole shot{shotKey.length > 0 && ` (${keysText(shotKey)})`}.
        </p>
      )}
    </Card>
  )
}

/** In or out point in the details panel: set it, or go to it. */
function MarkTile({
  label,
  slot,
  value,
  tc,
  onMark,
  onGo,
}: {
  label: string
  /** In (0) or out (1). */
  slot: number
  value: number | null
  tc: (f: number) => string
  onMark: () => void
  onGo: (f: number) => void
}) {
  const keys = useShortcutKeys()
  return (
    <div
      className={cn(
        "flex flex-col rounded-xl border px-3 py-2.5",
        value != null ? "border-accent-line bg-accent-subtle" : "border-white/[0.08] bg-black/20",
      )}
    >
      <div className="flex items-center justify-between">
        <span className={cn("text-xs font-semibold", value != null ? "text-accent" : "text-fg-3")}>{label}</span>
        <Kbd keys={keys("player.mark", slot)} />
      </div>
      {value != null ? (
        <Tooltip label="Go to this frame" side="left">
          <button onClick={() => onGo(value)} className="timecode mt-1.5 text-left text-md text-fg-1 hover:text-accent">
            {tc(value)}
          </button>
        </Tooltip>
      ) : (
        <button onClick={onMark} className="mt-1.5 text-left text-md text-fg-3 hover:text-fg-1">
          Mark {label.toLowerCase()}
        </button>
      )}
    </div>
  )
}
