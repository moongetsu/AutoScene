import { useId, type ReactNode } from "react"
import { RotateCcw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Slider } from "@/components/ui/slider"
import { cn } from "@/lib/utils"

/**
 * One titled block of settings. Rows inside are separated by hairlines so the
 * page reads as a list of decisions, not a wall of controls.
 */
export function SettingsGroup({
  title,
  description,
  aside,
  footer,
  children,
  className,
}: {
  title: string
  description?: string
  aside?: ReactNode
  footer?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={cn("panel-surface overflow-hidden", className)}>
      <header className="flex flex-wrap items-start justify-between gap-3 border-b border-border/40 px-5 py-4">
        <div className="min-w-0">
          <h2 className="font-heading text-base tracking-tight">{title}</h2>
          {description ? (
            <p className="mt-1 max-w-[64ch] text-xs leading-relaxed text-muted-foreground">
              {description}
            </p>
          ) : null}
        </div>
        {aside ? <div className="flex shrink-0 items-center gap-2">{aside}</div> : null}
      </header>
      <div className="divide-y divide-border/30">{children}</div>
      {footer ? (
        <div className="border-t border-border/40 bg-background/25 px-5 py-3">{footer}</div>
      ) : null}
    </section>
  )
}

/**
 * Label + plain-language hint on the left, control on the right.
 * `changed` marks a row edited since the last save.
 */
export function SettingRow({
  label,
  hint,
  htmlFor,
  changed,
  onReset,
  full,
  children,
}: {
  label: string
  hint?: ReactNode
  htmlFor?: string
  changed?: boolean
  onReset?: () => void
  /** Control spans the full row width (choice tiles, long lists). */
  full?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={cn(
        "grid gap-3 px-5 py-4 transition-colors",
        changed && "bg-primary/[0.035]",
        !full && "sm:grid-cols-[minmax(0,1fr)_minmax(0,17rem)] sm:items-center sm:gap-6",
      )}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span
            aria-hidden
            className={cn(
              "size-1.5 shrink-0 rounded-full transition-opacity",
              changed ? "bg-primary opacity-100" : "opacity-0",
            )}
          />
          <Label htmlFor={htmlFor} className="text-sm font-medium">
            {label}
          </Label>
          {onReset && changed ? (
            <Button
              type="button"
              variant="ghost"
              size="xs"
              className="text-muted-foreground"
              onClick={onReset}
            >
              <RotateCcw className="size-2.5" />
              Reset
            </Button>
          ) : null}
        </div>
        {hint ? (
          <p className="mt-1 max-w-[52ch] pl-3.5 text-xs leading-relaxed text-muted-foreground">
            {hint}
          </p>
        ) : null}
      </div>
      <div className={cn("min-w-0", full && "pl-0")}>{children}</div>
    </div>
  )
}

/** Big tappable tiles for a small set of named options. */
export function ChoiceTiles<T extends string>({
  value,
  options,
  onChange,
  columns = 2,
}: {
  value: T
  options: { id: T; label: string; blurb?: string; meta?: string; disabled?: boolean }[]
  onChange: (id: T) => void
  columns?: 2 | 3
}) {
  return (
    <div
      className={cn(
        "grid gap-2",
        columns === 3 ? "sm:grid-cols-3" : "sm:grid-cols-2",
      )}
      role="radiogroup"
    >
      {options.map((opt) => {
        const active = value === opt.id
        return (
          <button
            key={opt.id}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={opt.disabled}
            onClick={() => onChange(opt.id)}
            className={cn(
              "rounded-xl border px-4 py-3 text-left transition disabled:cursor-not-allowed disabled:opacity-50",
              active
                ? "border-primary/55 bg-primary/[0.07]"
                : "border-border/45 bg-background/30 hover:border-border hover:bg-background/50",
            )}
          >
            <div className="flex items-center justify-between gap-2">
              <p className="text-sm font-medium">{opt.label}</p>
              {active ? (
                <span className="text-[10px] uppercase tracking-[0.16em] text-primary">
                  On
                </span>
              ) : null}
            </div>
            {opt.blurb ? (
              <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{opt.blurb}</p>
            ) : null}
            {opt.meta ? (
              <p className="timecode mt-2 text-[11px] text-muted-foreground">{opt.meta}</p>
            ) : null}
          </button>
        )
      })}
    </div>
  )
}

/** Slider with the live value and end labels, so a number always has meaning. */
export function SliderControl({
  id,
  value,
  display,
  min,
  max,
  step = 1,
  minLabel,
  maxLabel,
  onChange,
}: {
  id: string
  value: number
  display: string
  min: number
  max: number
  step?: number
  minLabel?: string
  maxLabel?: string
  onChange: (n: number) => void
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <span className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
          {minLabel}
        </span>
        <span className="timecode text-sm tabular-nums text-primary">{display}</span>
        <span className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
          {maxLabel}
        </span>
      </div>
      <Slider
        id={id}
        min={min}
        max={max}
        step={step}
        value={[value]}
        onValueChange={(v) => {
          const n = Array.isArray(v) ? v[0] : v
          if (typeof n === "number") onChange(n)
        }}
      />
    </div>
  )
}

export function SelectControl({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: string
  options: { value: string; label: string; disabled?: boolean }[]
  onChange: (v: string) => void
  ariaLabel?: string
}) {
  return (
    <Select value={value} onValueChange={(v) => v && onChange(v)}>
      <SelectTrigger aria-label={ariaLabel} className="w-full bg-background/50">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

export function NumberControl({
  id,
  value,
  min,
  max,
  step,
  suffix,
  onChange,
}: {
  id: string
  value: number
  min: number
  max: number
  step?: number
  suffix?: string
  onChange: (n: number) => void
}) {
  return (
    <div className="flex items-center gap-2">
      <Input
        id={id}
        type="number"
        min={min}
        max={max}
        step={step}
        className="w-full bg-background/50"
        value={value}
        onChange={(e) => {
          const n = Number(e.target.value)
          onChange(Number.isFinite(n) && e.target.value !== "" ? n : value)
        }}
        onBlur={(e) => {
          const n = Number(e.target.value)
          if (!Number.isFinite(n)) return
          onChange(Math.min(max, Math.max(min, n)))
        }}
      />
      {suffix ? (
        <span className="shrink-0 text-xs text-muted-foreground">{suffix}</span>
      ) : null}
    </div>
  )
}

/** A read-only fact (paths, device, active pack). */
export function InfoRow({
  label,
  value,
  action,
  mono,
}: {
  label: string
  value: ReactNode
  action?: ReactNode
  mono?: boolean
}) {
  const id = useId()
  return (
    <div className="grid gap-2 px-5 py-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,17rem)] sm:items-center sm:gap-6">
      <p id={id} className="text-sm font-medium">
        {label}
      </p>
      <div className="flex min-w-0 items-center justify-between gap-2 sm:justify-end">
        <p
          aria-labelledby={id}
          className={cn(
            "min-w-0 break-all text-sm text-muted-foreground",
            mono && "font-mono text-xs leading-relaxed",
          )}
        >
          {value}
        </p>
        {action}
      </div>
    </div>
  )
}
