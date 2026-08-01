import { Layers3, Loader2, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  PACK_CONTEXT_BLURBS,
  PACK_CONTEXT_LABELS,
  type PackClips,
  type PackContextMode,
} from "@/lib/api"
import { cn } from "@/lib/utils"

const MODES: PackContextMode[] = ["face", "shot", "conversation", "scene"]

type Props = {
  pack: PackClips | null
  /** Wider modes need shot→scene grouping to exist. */
  hasSceneGroups: boolean
  busy?: boolean
  onModeChange: (mode: PackContextMode) => void
  onPadChange: (patch: { lead_shots?: number; tail_shots?: number }) => void
  onRebuildScenes: () => void
}

/**
 * How much context this character's pack keeps.
 *
 * Changes apply immediately — clips are widened when the pack is read, so
 * switching mode never costs a rescan.
 */
export function PackContextBar({
  pack,
  hasSceneGroups,
  busy = false,
  onModeChange,
  onPadChange,
  onRebuildScenes,
}: Props) {
  if (!pack) return null
  const needsGroups = (m: PackContextMode) =>
    (m === "conversation" || m === "scene") && !hasSceneGroups

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border/40 bg-card/30 px-2.5 py-2">
      <span className="inline-flex items-center gap-1 text-[10px] uppercase tracking-wider text-muted-foreground">
        <Layers3 className="size-3" />
        Keep
      </span>

      <div className="inline-flex overflow-hidden rounded-md border border-border/50">
        {MODES.map((m) => {
          const disabled = needsGroups(m) || busy
          return (
            <button
              key={m}
              type="button"
              disabled={disabled}
              onClick={() => onModeChange(m)}
              title={
                needsGroups(m)
                  ? "Needs scene grouping — rebuild scenes first"
                  : PACK_CONTEXT_BLURBS[m]
              }
              className={cn(
                "px-2.5 py-1 text-[11px] transition",
                pack.context_mode === m
                  ? "bg-primary/15 text-primary"
                  : "text-muted-foreground hover:text-foreground",
                disabled && pack.context_mode !== m && "opacity-40",
              )}
            >
              {PACK_CONTEXT_LABELS[m]}
            </button>
          )
        })}
      </div>

      {pack.context_mode !== "face" ? (
        <div className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <ShotPad
            label="lead"
            value={pack.lead_shots}
            busy={busy}
            onChange={(v) => onPadChange({ lead_shots: v })}
          />
          <ShotPad
            label="tail"
            value={pack.tail_shots}
            busy={busy}
            onChange={(v) => onPadChange({ tail_shots: v })}
          />
        </div>
      ) : null}

      <span className="text-[11px] text-muted-foreground">
        {pack.clips.length} clip{pack.clips.length === 1 ? "" : "s"}
        {pack.missing_shot_indices.length ? (
          <span className="ml-1.5 text-[#C4A35A]">
            · {pack.missing_shot_indices.length} detected shot
            {pack.missing_shot_indices.length === 1 ? "" : "s"} not included
          </span>
        ) : null}
      </span>

      <div className="ml-auto flex items-center gap-1.5">
        {busy ? <Loader2 className="size-3 animate-spin text-muted-foreground" /> : null}
        <Button
          size="sm"
          variant="ghost"
          className="h-6 gap-1 text-[11px]"
          disabled={busy}
          onClick={onRebuildScenes}
          title={
            hasSceneGroups
              ? "Regroup shots into scenes from the current cast"
              : "Group shots into scenes to unlock Conversation and Full scene"
          }
        >
          <RefreshCw className="size-3" />
          {hasSceneGroups ? "Rebuild scenes" : "Group scenes"}
        </Button>
      </div>
    </div>
  )
}

function ShotPad({
  label,
  value,
  busy,
  onChange,
}: {
  label: string
  value: number
  busy: boolean
  onChange: (v: number) => void
}) {
  return (
    <span className="inline-flex items-center gap-0.5 rounded border border-border/40 px-1">
      <button
        type="button"
        aria-label={`Decrease ${label} shots`}
        disabled={busy || value <= 0}
        onClick={() => onChange(Math.max(0, value - 1))}
        className="px-1 disabled:opacity-30 hover:text-foreground"
      >
        −
      </button>
      <span className="tabular-nums">
        {value > 0 ? `+${value}` : "0"} {label}
      </span>
      <button
        type="button"
        aria-label={`Increase ${label} shots`}
        disabled={busy || value >= 10}
        onClick={() => onChange(Math.min(10, value + 1))}
        className="px-1 disabled:opacity-30 hover:text-foreground"
      >
        +
      </button>
    </span>
  )
}
