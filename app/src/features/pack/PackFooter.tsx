import { ArrowCounterClockwise, Package } from "@/components/icons"
import type { JobInfo, PackTotals } from "@/bindings"
import { Button } from "@/components/ui/button"
import { ProgressRing } from "@/components/ui/progress"
import { Tooltip } from "@/components/ui/tooltip"
import { plural } from "@/features/cast/cast"
import { formatBytes, formatDuration, formatPercent } from "@/lib/format"
import { modKey } from "@/lib/platform"

/**
 * The pack at a glance, floating over the shots: how many, how long, about
 * how big in the format an export starts with, the picks by hand (to hand
 * back to the filters) and Export. While the pack's export waits or runs,
 * Export shows how far it is and opens Activity instead: a second click
 * doesn't queue the same pack again.
 */
export function PackFooter({
  totals,
  bytes,
  format,
  picks,
  canExport,
  exporting,
  onClearPicks,
  onExport,
  onShowExport,
}: {
  totals: PackTotals
  /** In `format`, the one an export starts with. */
  bytes: number | null
  /** The format's short name. */
  format: string | null
  picks: number
  canExport: boolean
  /** The job of the pack's export while it waits or runs. */
  exporting?: Pick<JobInfo, "state" | "progress">
  onClearPicks: () => void
  onExport: () => void
  /** Show the export's progress (in Activity). */
  onShowExport: () => void
}) {
  return (
    <div className="glass-strong absolute bottom-6 left-1/2 z-30 flex -translate-x-1/2 animate-pop-in items-center gap-1 rounded-full py-1.5 pr-1.5 pl-5 whitespace-nowrap">
      <span className="text-md font-semibold text-fg-1">{plural(totals.clips, "shot")}</span>
      <span className="pr-2 pl-1.5 text-sm text-fg-3">
        {formatDuration(totals.seconds ?? 0)}
        {bytes != null && bytes > 0 && ` · about ${formatBytes(bytes)}${format ? ` (${format})` : ""}`}
      </span>
      {picks > 0 && (
        <Tooltip label="Hand every shot back to the filters">
          <Button size="sm" variant="ghost" onClick={onClearPicks}>
            <ArrowCounterClockwise /> {plural(picks, "pick")} by hand
          </Button>
        </Tooltip>
      )}
      {exporting ? (
        <Tooltip label="Show it in Activity">
          <Button size="md" variant="accent" onClick={onShowExport}>
            <ProgressRing aria-hidden value={exporting.state === "running" ? exporting.progress : null} />
            {exporting.state === "queued" ? (
              "Export queued"
            ) : (
              <span>
                Exporting… <span className="timecode">{formatPercent(exporting.progress)}</span>
              </span>
            )}
          </Button>
        </Tooltip>
      ) : (
        <Tooltip label="Export the pack" shortcut={[modKey, "E"]}>
          <Button size="md" variant="primary" disabled={!canExport} onClick={onExport}>
            <Package weight="bold" /> Export
          </Button>
        </Tooltip>
      )}
    </div>
  )
}
