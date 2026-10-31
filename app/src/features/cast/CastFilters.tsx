import { ArrowsDownUp, CaretDown, MagnifyingGlass, X } from "@/components/icons"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Segmented } from "@/components/ui/segmented"
import { cn } from "@/lib/utils"
import type { CastFind, CastOrder, CastShow } from "./cast"

const ORDERS: { value: CastOrder; label: string }[] = [
  { value: "screen", label: "Most screen time" },
  { value: "shots", label: "Most shots" },
  { value: "name", label: "By name" },
]

/** Finding someone in a big cast: by name, only the named or those to review, in another order. */
export function CastFilters({
  find,
  review,
  onChange,
  className,
}: {
  find: CastFind
  /** Someone has faces to review: else "To review" isn't offered. */
  review: boolean
  onChange: (find: CastFind) => void
  className?: string
}) {
  const set = (patch: Partial<CastFind>) => onChange({ ...find, ...patch })
  const shows: { value: CastShow; label: string }[] = [
    { value: "all", label: "All" },
    { value: "named", label: "Named" },
    // Kept while chosen, so the choice still shows once the last face is reviewed.
    ...(review || find.show === "review" ? [{ value: "review" as const, label: "To review" }] : []),
  ]
  const order = ORDERS.find((o) => o.value === find.order) ?? ORDERS[0]
  return (
    <div role="group" aria-label="Find" className={cn("flex flex-wrap items-center gap-2.5", className)}>
      <div className="relative">
        <MagnifyingGlass className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-fg-3" />
        <Input
          value={find.query}
          onChange={(e) => set({ query: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Escape" && find.query) set({ query: "" })
          }}
          placeholder="Find a character"
          aria-label="Find a character"
          className="h-9 w-64 rounded-full pr-9 pl-10 text-sm"
        />
        {find.query && (
          <button
            type="button"
            onClick={() => set({ query: "" })}
            aria-label="Clear the search"
            className="absolute top-1/2 right-1.5 flex size-6 -translate-y-1/2 items-center justify-center rounded-full text-fg-3 transition-colors hover:bg-hover hover:text-fg-1"
          >
            <X className="size-3.5" />
          </button>
        )}
      </div>
      <Segmented<CastShow> aria-label="Show" value={find.show} onChange={(show) => set({ show })} options={shows} />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" aria-label={`Order: ${order.label}`}>
            <ArrowsDownUp /> {order.label} <CaretDown className="opacity-70" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[200px]">
          <DropdownMenuRadioGroup value={find.order} onValueChange={(v) => set({ order: v as CastOrder })}>
            {ORDERS.map((o) => (
              <DropdownMenuRadioItem key={o.value} value={o.value}>
                {o.label}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}
