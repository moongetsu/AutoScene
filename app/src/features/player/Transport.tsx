import type * as React from "react"
import { useEffect, useRef } from "react"
import {
  CaretLineLeft,
  CaretLineRight,
  Pause,
  Play,
  Scissors,
  SkipBack,
  SkipForward,
  SpeakerHigh,
  SpeakerLow,
  SpeakerSlash,
} from "@/components/icons"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Tooltip } from "@/components/ui/tooltip"
import { modKey } from "@/lib/platform"
import { cn } from "@/lib/utils"
import { type FrameStore, useFrame } from "./frame-store"
import { rateLabel } from "./shuttle"

/**
 * Under the picture: the timecode (click or Ctrl/⌘ G to go somewhere), the
 * transport, in and out, Export and the volume.
 */
export function Transport({
  store,
  last,
  rate,
  tc,
  parse,
  goto,
  onGoto,
  onSeek,
  onStep,
  onToggle,
  inFrame,
  outFrame,
  onMarkIn,
  onMarkOut,
  showExport,
  canExport,
  onExport,
  volume,
  muted,
  onVolume,
  onToggleMute,
}: {
  store: FrameStore
  last: number
  rate: number
  tc: (f: number) => string
  /** A typed timecode or frame number as a frame (relative to the one shown), or null. */
  parse: (text: string, from: number) => number | null
  goto: boolean
  onGoto: (open: boolean) => void
  onSeek: (f: number) => void
  onStep: (delta: number) => void
  onToggle: () => void
  inFrame: number | null
  outFrame: number | null
  onMarkIn: () => void
  onMarkOut: () => void
  showExport: boolean
  canExport: boolean
  onExport: () => void
  volume: number
  muted: boolean
  onVolume: (v: number) => void
  onToggleMute: () => void
}) {
  const playing = rate !== 0
  const shuttle = rateLabel(rate)
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 pb-4">
      <div className="flex min-w-0 items-baseline gap-3">
        <FrameReadout store={store} last={last} tc={tc} parse={parse} goto={goto} onGoto={onGoto} onSeek={onSeek} />
        {shuttle && (
          <span className="timecode shrink-0 rounded-full border border-accent-line bg-accent-subtle px-2 text-xs leading-5 text-accent">
            {shuttle}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1">
        <TransportButton label="First frame" keys={["Home"]} onClick={() => onSeek(0)}>
          <SkipBack weight="fill" />
        </TransportButton>
        <TransportButton label="Previous frame" keys={["←"]} onClick={() => onStep(-1)}>
          <CaretLineLeft weight="bold" />
        </TransportButton>
        <Tooltip label={playing ? "Pause" : "Play"} shortcut={["Space"]}>
          <button
            onClick={onToggle}
            aria-label={playing ? "Pause" : "Play"}
            className="mx-2 flex size-12 items-center justify-center rounded-full bg-cta text-on-cta shadow-[0_10px_30px_-8px_rgb(255_255_255/0.35)] transition-transform duration-[var(--dur-fast)] hover:scale-105 active:scale-95 [&_svg]:size-5"
          >
            {playing ? <Pause weight="fill" /> : <Play weight="fill" className="translate-x-px" />}
          </button>
        </Tooltip>
        <TransportButton label="Next frame" keys={["→"]} onClick={() => onStep(1)}>
          <CaretLineRight weight="bold" />
        </TransportButton>
        <TransportButton label="Last frame" keys={["End"]} onClick={() => onSeek(last)}>
          <SkipForward weight="fill" />
        </TransportButton>
      </div>

      <div className="flex min-w-0 items-center justify-end gap-2">
        <MarkChip label="In" value={inFrame} tc={tc} onMark={onMarkIn} onGo={onSeek} keyHint="I" />
        <MarkChip label="Out" value={outFrame} tc={tc} onMark={onMarkOut} onGo={onSeek} keyHint="O" />
        {showExport && (
          <Tooltip label="Export clip" shortcut={[modKey, "E"]}>
            <Button size="icon" variant="primary" aria-label="Export clip" disabled={!canExport} onClick={onExport}>
              <Scissors weight="bold" />
            </Button>
          </Tooltip>
        )}
        <div className="ml-1 hidden items-center gap-1.5 @3xl:flex">
          <Tooltip label={muted ? "Unmute" : "Mute"} shortcut={["M"]}>
            <button
              onClick={onToggleMute}
              aria-label={muted ? "Unmute" : "Mute"}
              className="flex size-9 items-center justify-center rounded-full text-fg-2 transition-colors hover:bg-white/[0.08] hover:text-fg-1 [&_svg]:size-[18px]"
            >
              {muted || volume === 0 ? <SpeakerSlash /> : volume < 0.5 ? <SpeakerLow /> : <SpeakerHigh />}
            </button>
          </Tooltip>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={muted ? 0 : volume}
            aria-label="Volume"
            onChange={(e) => onVolume(Number(e.currentTarget.value))}
            className="h-1 w-20 accent-[var(--fg-1)]"
          />
        </div>
      </div>
    </div>
  )
}

/** The timecode of the frame on screen — the one part that draws on every frame. */
function FrameReadout({
  store,
  last,
  tc,
  parse,
  goto,
  onGoto,
  onSeek,
}: {
  store: FrameStore
  last: number
  tc: (f: number) => string
  parse: (text: string, from: number) => number | null
  goto: boolean
  onGoto: (open: boolean) => void
  onSeek: (f: number) => void
}) {
  const frame = useFrame(store)
  const field = useRef<HTMLInputElement>(null)
  // Select the go-to field once when it opens, not on every frame (that
  // would wipe what is being typed).
  useEffect(() => {
    if (!goto) return
    field.current?.focus()
    field.current?.select()
  }, [goto])
  return (
    <>
      {goto ? (
        <input
          ref={field}
          aria-label="Go to timecode or frame"
          defaultValue={tc(frame)}
          spellCheck={false}
          className="timecode h-10 w-[150px] rounded-xl border border-accent-line bg-white/[0.06] px-3 text-lg text-fg-1 outline-none @3xl:w-[190px] @3xl:text-2xl"
          onBlur={() => onGoto(false)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onGoto(false)
            if (e.key !== "Enter") return
            const f = parse(e.currentTarget.value, store.get())
            if (f == null) {
              toast.error("That isn't a timecode or frame number", {
                description: "Try 00:12:03:05, a frame like 17099, or +48 to jump ahead.",
              })
              return
            }
            onSeek(f)
            onGoto(false)
          }}
        />
      ) : (
        <Tooltip label="Go to timecode or frame" shortcut={[modKey, "G"]}>
          <button
            onClick={() => onGoto(true)}
            className="timecode -mx-2 rounded-xl px-2 text-lg font-medium tracking-tight text-fg-1 transition-colors hover:bg-white/[0.06] @3xl:text-2xl"
          >
            {tc(frame)}
          </button>
        </Tooltip>
      )}
      <span className="timecode hidden text-sm whitespace-nowrap text-fg-3 @xl:inline">
        {frame} / {last}
      </span>
    </>
  )
}

function TransportButton({
  label,
  keys,
  onClick,
  children,
}: {
  label: string
  keys: string[]
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <Tooltip label={label} shortcut={keys}>
      <button
        onClick={onClick}
        aria-label={label}
        className="flex size-10 items-center justify-center rounded-full text-fg-2 transition-colors hover:bg-white/[0.08] hover:text-fg-1 [&_svg]:size-[18px]"
      >
        {children}
      </button>
    </Tooltip>
  )
}

/** In or out point in the transport row: gold once set. */
function MarkChip({
  label,
  keyHint,
  value,
  tc,
  onMark,
  onGo,
}: {
  label: string
  keyHint: string
  value: number | null
  tc: (f: number) => string
  onMark: () => void
  onGo: (f: number) => void
}) {
  return (
    <Tooltip
      label={value != null ? `Go to ${label.toLowerCase()} point` : `Mark ${label.toLowerCase()}`}
      shortcut={value != null ? ["Shift", keyHint] : [keyHint]}
    >
      <button
        onClick={value != null ? () => onGo(value) : onMark}
        className={cn(
          "flex h-9 items-center gap-2 rounded-full border px-3 text-sm whitespace-nowrap transition-colors",
          value != null
            ? "border-accent-line bg-accent-subtle text-fg-1 hover:bg-accent/25"
            : "border-white/10 text-fg-3 hover:border-white/20 hover:text-fg-1",
        )}
      >
        <span className={cn("text-xs font-semibold", value != null && "text-accent")}>{label}</span>
        {value != null && <span className="timecode hidden @5xl:inline">{tc(value)}</span>}
      </button>
    </Tooltip>
  )
}
