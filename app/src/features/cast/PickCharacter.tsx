import { useState } from "react"
import { UserPlus } from "@/components/icons"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { assetUrl } from "@/lib/media"
import { useCast } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { formatScreenTime, plural } from "./cast"

export type Pick = { kind: "existing"; id: string } | { kind: "new"; name: string }

/** Choose a character (or start a new one), optionally confirming the choice. */
export function PickCharacter({
  pid,
  open,
  onOpenChange,
  title,
  description,
  exclude,
  allowNew,
  confirm,
  onPick,
}: {
  pid: string
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  exclude?: string
  allowNew?: boolean
  /** Ask before an existing character is taken (for what can't be undone). */
  confirm?: (target: Character) => { question: string; detail: string; action: string }
  onPick: (p: Pick) => void
}) {
  const { data: cast } = useCast(pid)
  const [query, setQuery] = useState("")
  // The row arrows moved to (an index into `options`); null: none yet.
  const [active, setActive] = useState<number | null>(null)
  const [pending, setPending] = useState<Character | null>(null)
  const q = query.trim().toLowerCase()
  const options = (cast?.characters ?? []).filter((c) => c.id !== exclude && (!q || c.label.toLowerCase().includes(q)))
  // Enter takes the row the arrows chose, else an exact name — never just
  // the first partial match (typing a new name must not pick an old one).
  const exact = options.findIndex((c) => c.label.toLowerCase() === q)
  const selected = active ?? (exact >= 0 ? exact : null)
  const reset = () => {
    setQuery("")
    setActive(null)
    setPending(null)
  }
  const choose = (c: Character) => (confirm ? setPending(c) : onPick({ kind: "existing", id: c.id }))
  const asked = pending && confirm?.(pending)
  return (
    <Dialog open={open} onOpenChange={(o) => (onOpenChange(o), reset())}>
      <DialogContent title={title} description={description} className="w-[min(440px,calc(100vw-32px))]">
        {pending && asked ? (
          <div className="flex flex-col gap-4">
            <div className="flex items-center gap-3">
              <span className="size-14 shrink-0 overflow-hidden rounded-full bg-inset picture-edge">
                {pending.coverSmall && <img src={assetUrl(pending.coverSmall)} alt="" className="size-full object-cover" />}
              </span>
              <div>
                <p className="text-md font-medium text-fg-1">{asked.question}</p>
                <p className="mt-1 text-sm text-fg-3">{asked.detail}</p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setPending(null)}>
                Back
              </Button>
              <Button variant="primary" autoFocus onClick={() => onPick({ kind: "existing", id: pending.id })}>
                {asked.action}
              </Button>
            </div>
          </div>
        ) : (
          <>
            <Input
              autoFocus
              placeholder={allowNew ? "Search, or type a new name" : "Search"}
              value={query}
              role="combobox"
              aria-expanded
              aria-controls="pick-character-list"
              aria-activedescendant={selected != null && options[selected] ? `pick-${options[selected].id}` : undefined}
              onChange={(e) => {
                setQuery(e.target.value)
                setActive(null)
              }}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault()
                  if (options.length === 0) return
                  const from = selected ?? (e.key === "ArrowDown" ? -1 : options.length)
                  setActive(Math.min(options.length - 1, Math.max(0, from + (e.key === "ArrowDown" ? 1 : -1))))
                } else if (e.key === "Enter") {
                  e.preventDefault()
                  if (selected != null && options[selected]) choose(options[selected])
                  else if (allowNew && q) onPick({ kind: "new", name: query.trim() })
                }
              }}
            />
            <div id="pick-character-list" role="listbox" className="-mx-2 mt-3 max-h-[360px] overflow-y-auto px-1">
              {allowNew && (
                <button
                  onClick={() => onPick({ kind: "new", name: query.trim() })}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left text-md text-fg-1 hover:bg-hover",
                    selected == null && q && "bg-hover",
                  )}
                >
                  <span className="flex size-10 items-center justify-center rounded-full border border-dashed border-white/25 text-fg-3">
                    <UserPlus className="size-4" />
                  </span>
                  {query.trim() ? <>New character “{query.trim()}”</> : "New character"}
                </button>
              )}
              {options.map((c, i) => (
                <button
                  key={c.id}
                  id={`pick-${c.id}`}
                  role="option"
                  aria-selected={i === selected}
                  onClick={() => choose(c)}
                  onMouseMove={() => active !== i && setActive(i)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left hover:bg-hover",
                    i === selected && "bg-active",
                  )}
                >
                  <span className="size-10 shrink-0 overflow-hidden rounded-full bg-inset picture-edge">
                    {c.coverSmall && <img src={assetUrl(c.coverSmall)} alt="" className="size-full object-cover" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className={cn("block truncate text-md font-medium", c.name ? "text-fg-1" : "text-fg-2")}>{c.label}</span>
                    <span className="block text-xs text-fg-3">
                      {formatScreenTime(c.screenSec ?? 0)} · {plural(c.shots, "shot")}
                    </span>
                  </span>
                </button>
              ))}
              {!allowNew && options.length === 0 && <p className="px-3 py-4 text-md text-fg-3">Nobody matches.</p>}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
