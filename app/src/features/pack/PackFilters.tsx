import type * as React from "react"
import { ArrowCounterClockwise, CaretDown } from "@/components/icons"
import type { MediaItem, PackOrder, PackSpec, ShotSize } from "@/bindings"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Segmented } from "@/components/ui/segmented"
import { Tooltip } from "@/components/ui/tooltip"
import { mediaTitle } from "@/lib/media"
import { cn } from "@/lib/utils"
import { DEFAULT_SPEC, HANDLES, LENGTHS, LIMITS, SIZES, isDefault, lengthOf } from "./spec"

/** The pack's filters, in a bar that stays under the top bar while the shots scroll. */
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
    set({ sizes: spec.sizes.includes(s) ? spec.sizes.filter((x) => x !== s) : SIZES.map((x) => x.value).filter((x) => x === s || spec.sizes.includes(x)) })
  const length = lengthOf(spec)
  const episodes = spec.media.length === 0 ? media.length : spec.media.filter((id) => media.some((m) => m.id === id)).length

  return (
    <div
      role="group"
      aria-label="Filters"
      className="sticky top-[var(--topbar-h)] z-10 flex flex-wrap items-center gap-2 border-b border-hairline bg-canvas/80 px-10 py-3 backdrop-blur-2xl backdrop-saturate-150"
    >
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
      <div role="group" aria-label="Shot sizes" className="flex items-center gap-1">
        {SIZES.map((s) => (
          <Tooltip key={s.value} label={s.label}>
            <Chip pressed={spec.sizes.includes(s.value)} onClick={() => toggleSize(s.value)} aria-label={s.label}>
              {s.short}
            </Chip>
          </Tooltip>
        ))}
      </div>
      <Divider />
      <Menu label={length.label} active={spec.minSec != null || spec.maxSec != null}>
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
      </Menu>
      {media.length > 1 && (
        <Menu
          label={spec.media.length === 0 ? "All episodes" : `${episodes} of ${media.length} episodes`}
          active={spec.media.length > 0}
        >
          <DropdownMenuItem onSelect={() => set({ media: [] })} disabled={spec.media.length === 0}>
            All episodes
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <div className="max-h-[min(420px,60vh)] overflow-y-auto">
            {media.map((m) => {
              const on = spec.media.length === 0 || spec.media.includes(m.id)
              const only = on && episodes === 1
              return (
                <DropdownMenuCheckboxItem
                  key={m.id}
                  checked={on}
                  // One episode at least: none would mean all again.
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
        </Menu>
      )}
      <Menu label={spec.limit == null ? "No limit" : `Best ${spec.limit}`} active={spec.limit != null}>
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
      </Menu>
      <Menu label={spec.handles === 0 ? "No handles" : `Handles ${spec.handles} fr`} active={spec.handles > 0}>
        <DropdownMenuRadioGroup value={String(spec.handles)} onValueChange={(v) => set({ handles: Number(v) })}>
          {HANDLES.map((n) => (
            <DropdownMenuRadioItem key={n} value={String(n)}>
              {n === 0 ? "No handles" : `${n} frames before and after`}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </Menu>
      <Divider />
      <Chip pressed={spec.solo} onClick={() => set({ solo: !spec.solo })}>
        Only them in the shot
      </Chip>
      <Chip pressed={spec.skipTiny} onClick={() => set({ skipTiny: !spec.skipTiny })}>
        Skip flash cuts
      </Chip>
      {(faceless || spec.skipFaceless) && (
        <Tooltip label="Leave out the shots where their face doesn't show (found by their body)">
          <Chip pressed={spec.skipFaceless ?? false} onClick={() => set({ skipFaceless: !spec.skipFaceless })}>
            Only with their face
          </Chip>
        </Tooltip>
      )}
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
