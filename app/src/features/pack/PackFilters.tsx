import type * as React from "react"
import { ArrowCounterClockwise, CaretDown, Check, FunnelSimple } from "@/components/icons"
import type { MediaItem, PackOrder, PackSpec, ShotSize } from "@/bindings"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Segmented } from "@/components/ui/segmented"
import { mediaTitle } from "@/lib/media"
import { cn } from "@/lib/utils"
import { DEFAULT_SPEC, HANDLES, LENGTHS, LIMITS, SIZES, isDefault, lengthOf } from "./spec"

/** Count active filters that diverge from the default spec. */
function activeFiltersCount(spec: PackSpec, totalMediaCount: number): number {
  let count = 0
  if (spec.sizes.length > 0) count++
  if (spec.minSec != null || spec.maxSec != null) count++
  if (spec.media.length > 0 && spec.media.length < totalMediaCount) count++
  if (spec.limit != null) count++
  if (spec.solo) count++
  if (!spec.skipTiny) count++
  if (spec.skipFaceless) count++
  return count
}

/** The pack's filters, in a streamlined bar that stays under the top bar while the shots scroll. */
export function PackFilters({
  spec,
  media,
  faceless,
  onChange,
}: {
  spec: PackSpec
  media: MediaItem[]
  /** The pack has shots where the character's face doesn't show. */
  faceless: boolean
  onChange: (spec: PackSpec) => void
}) {
  const set = (patch: Partial<PackSpec>) => onChange({ ...spec, ...patch })
  const toggleSize = (s: ShotSize) =>
    set({
      sizes: spec.sizes.includes(s)
        ? spec.sizes.filter((x) => x !== s)
        : SIZES.map((x) => x.value).filter((x) => x === s || spec.sizes.includes(x)),
    })
  const length = lengthOf(spec)
  const episodes = spec.media.length === 0 ? media.length : spec.media.filter((id) => media.some((m) => m.id === id)).length
  const filterCount = activeFiltersCount(spec, media.length)

  return (
    <div
      role="group"
      aria-label="Filters"
      className="sticky top-[var(--topbar-h)] z-10 flex flex-wrap items-center gap-3 border-b border-hairline bg-canvas/80 px-10 py-3 backdrop-blur-2xl backdrop-saturate-150"
    >
      {/* 1. Primary Order Switcher */}
      <Segmented<PackOrder>
        aria-label="Order"
        value={spec.order}
        onChange={(order) => set({ order })}
        options={[
          { value: "episodes", label: "Episode order" },
          { value: "best", label: "Best first" },
        ]}
      />

      <Divider />

      {/* 2. Consolidated Filters Menu */}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Chip pressed={filterCount > 0} aria-haspopup="menu" aria-pressed={undefined} className="gap-2">
            <FunnelSimple className="size-4" />
            <span>Filters</span>
            {filterCount > 0 && (
              <span className="flex size-4 items-center justify-center rounded-full bg-accent text-[11px] font-bold text-accent-contrast">
                {filterCount}
              </span>
            )}
            <CaretDown className="size-3.5 opacity-70" />
          </Chip>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-72 p-2">
          {/* Shot sizes section */}
          <div className="px-2 py-1.5 text-xs font-semibold uppercase tracking-wider text-fg-3">
            Shot Sizes
          </div>
          <div className="flex flex-wrap items-center gap-1 px-2 pb-2">
            {SIZES.map((s) => {
              const active = spec.sizes.includes(s.value)
              return (
                <button
                  key={s.value}
                  type="button"
                  onClick={(e) => {
                    e.preventDefault()
                    toggleSize(s.value)
                  }}
                  className={cn(
                    "flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium transition-colors",
                    active
                      ? "bg-accent-subtle text-fg-1 ring-1 ring-accent-line"
                      : "bg-white/[0.04] text-fg-3 hover:bg-white/[0.08] hover:text-fg-1",
                  )}
                >
                  {active && <Check className="size-3 text-accent" />}
                  <span>{s.short}</span>
                </button>
              )
            })}
          </div>

          <DropdownMenuSeparator />

          {/* Duration submenu */}
          <div>
            <DropdownMenuLabel>Duration</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={length.id ?? ""}
              onValueChange={(id) => {
                const l = LENGTHS.find((x) => x.id === id)
                if (l) set({ minSec: l.min, maxSec: l.max })
              }}
            >
              {LENGTHS.map((l) => (
                <DropdownMenuRadioItem key={l.id} value={l.id}>
                  {l.label}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </div>

          <DropdownMenuSeparator />

          {/* Rules section */}
          <div>
            <DropdownMenuLabel>Rules</DropdownMenuLabel>
            <DropdownMenuCheckboxItem
              checked={spec.solo}
              onCheckedChange={(checked) => set({ solo: checked === true })}
            >
              Only them in the shot
            </DropdownMenuCheckboxItem>
            <DropdownMenuCheckboxItem
              checked={spec.skipTiny}
              onCheckedChange={(checked) => set({ skipTiny: checked === true })}
            >
              Skip flash cuts
            </DropdownMenuCheckboxItem>
            {(faceless || spec.skipFaceless) && (
              <DropdownMenuCheckboxItem
                checked={spec.skipFaceless ?? false}
                onCheckedChange={(checked) => set({ skipFaceless: checked === true })}
              >
                Only with their face
              </DropdownMenuCheckboxItem>
            )}
          </div>

          <DropdownMenuSeparator />

          {/* Limits section */}
          <div>
            <DropdownMenuLabel>Limit</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={String(spec.limit)}
              onValueChange={(v) => set({ limit: v === "null" ? null : Number(v) })}
            >
              {LIMITS.map((n) => (
                <DropdownMenuRadioItem key={String(n)} value={String(n)}>
                  {n == null ? "No limit" : `The best ${n}`}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </div>

          {/* Multi-episode selection when more than 1 exists */}
          {media.length > 1 && (
            <>
              <DropdownMenuSeparator />
              <div>
                <div className="flex items-center justify-between px-2 py-1.5">
                  <span className="text-xs font-semibold uppercase tracking-wider text-fg-3">
                    Episodes ({episodes}/{media.length})
                  </span>
                  {spec.media.length > 0 && (
                    <button
                      type="button"
                      onClick={() => set({ media: [] })}
                      className="text-[11px] text-accent hover:underline"
                    >
                      All
                    </button>
                  )}
                </div>
                <div className="max-h-48 overflow-y-auto">
                  {media.map((m) => {
                    const on = spec.media.length === 0 || spec.media.includes(m.id)
                    const only = on && episodes === 1
                    return (
                      <DropdownMenuCheckboxItem
                        key={m.id}
                        checked={on}
                        disabled={only}
                        onCheckedChange={(checked) => {
                          const ids = spec.media.length === 0 ? media.map((x) => x.id) : spec.media
                          const next = checked ? [...ids, m.id] : ids.filter((id) => id !== m.id)
                          const all = media.every((x) => next.includes(x.id))
                          set({ media: all ? [] : media.map((x) => x.id).filter((id) => next.includes(id)) })
                        }}
                      >
                        <span className="truncate">{mediaTitle(m)}</span>
                      </DropdownMenuCheckboxItem>
                    )
                  })}
                </div>
              </div>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      {/* 3. Export Handles Parameter */}
      <Menu label={spec.handles === 0 ? "No handles" : `Handles ${spec.handles} fr`} active={spec.handles > 0}>
        <DropdownMenuRadioGroup value={String(spec.handles)} onValueChange={(v) => set({ handles: Number(v) })}>
          {HANDLES.map((n) => (
            <DropdownMenuRadioItem key={n} value={String(n)}>
              {n === 0 ? "No handles" : `${n} frames before and after`}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </Menu>

      {/* 4. Reset Filters Action */}
      {!isDefault(spec) && (
        <Button size="sm" variant="ghost" className="ml-auto" onClick={() => onChange(DEFAULT_SPEC)}>
          <ArrowCounterClockwise /> Reset filters
        </Button>
      )}
    </div>
  )
}

function Divider() {
  return <span aria-hidden className="mx-1 h-5 w-px bg-white/10" />
}

function Chip({
  pressed,
  className,
  ...props
}: React.ComponentProps<"button"> & { pressed: boolean }) {
  return (
    <button
      type="button"
      aria-pressed={pressed}
      className={cn(
        "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-sm font-medium transition-[color,background-color,border-color] duration-[var(--dur-fast)]",
        pressed
          ? "border-accent-line bg-accent-subtle text-fg-1"
          : "border-white/10 bg-white/[0.04] text-fg-2 hover:border-white/20 hover:text-fg-1",
        className,
      )}
      {...props}
    />
  )
}

function Menu({ label, active, children }: { label: string; active: boolean; children: React.ReactNode }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Chip pressed={active} aria-haspopup="menu" aria-pressed={undefined}>
          {label} <CaretDown className="size-3.5 opacity-70" />
        </Chip>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-[220px]">
        {children}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
