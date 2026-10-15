import { ArrowCounterClockwise, Package } from "@/components/icons"
import type { PackTotals } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Tooltip } from "@/components/ui/tooltip"
import { plural } from "@/features/cast/cast"
import { formatBytes, formatDuration } from "@/lib/format"
import { modKey } from "@/lib/platform"

/**
 * The pack at a glance, floating over the shots: how many, how long, about
 * how big, the picks by hand (to hand back to the filters) and Export.
 */
export function PackFooter({
  totals,
  bytes,
  picks,
  canExport,
  onClearPicks,
  onExport,
}: {
  totals: PackTotals
  /** With the preset an export would use. */
  bytes: number | null
  picks: number
  canExport: boolean
  onClearPicks: () => void
  onExport: () => void
}) {
  return (
    <div className="glass-strong absolute bottom-6 left-1/2 z-30 flex -translate-x-1/2 animate-pop-in items-center gap-1 rounded-full py-1.5 pr-1.5 pl-5 whitespace-nowrap">
      <span className="text-md font-semibold text-fg-1">{plural(totals.clips, "shot")}</span>
      <span className="pr-2 pl-1.5 text-sm text-fg-3">
        {formatDuration(totals.seconds ?? 0)}
        {bytes != null && bytes > 0 && ` · about ${formatBytes(bytes)}`}
      </span>
      {picks > 0 && (
        <Tooltip label="Hand every shot back to the filters">
          <Button size="sm" variant="ghost" onClick={onClearPicks}>
            <ArrowCounterClockwise /> {plural(picks, "pick")} by hand
          </Button>
        </Tooltip>
      )}
      <Tooltip label="Export the pack" shortcut={[modKey, "E"]}>
        <Button size="md" variant="primary" disabled={!canExport} onClick={onExport}>
          <Package weight="bold" /> Export
        </Button>
      </Tooltip>
    </div>
  )
}
