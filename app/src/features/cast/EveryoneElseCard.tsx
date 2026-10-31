import { Link, useNavigate } from "@tanstack/react-router"
import { User } from "@/components/icons"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { plural } from "./cast"

/** Reusable card / row for "Everyone else" (unassigned faces) across Grid and List views. */
export function EveryoneElseCard({
  pid,
  unassigned,
  view = "grid",
  compact = false,
}: {
  pid: string
  unassigned: number
  view?: "grid" | "list"
  compact?: boolean
}) {
  const navigate = useNavigate()
  if (unassigned <= 0) return null

  // An item that navigates, not one wrapping a link: the item adds its
  // shortcut beside its children, and a Slot takes a single child only.
  const menu = (
    <ContextMenuItem onSelect={() => void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: "unassigned" } })}>
      <User /> Open everyone else
    </ContextMenuItem>
  )

  if (view === "list") {
    return (
      <ContextArea menu={menu}>
        <Link
          to="/p/$pid/cast/$cid"
          params={{ pid, cid: "unassigned" }}
          className="flex items-center justify-between gap-4 rounded-xl px-4 py-2.5 transition-colors hover:bg-white/[0.04]"
        >
          <div className="flex items-center gap-3.5">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-dashed border-white/20 text-fg-3">
              <User />
            </div>
            <div>
              <div className="text-md font-medium text-fg-1">Everyone else</div>
              <div className="text-xs text-fg-3">{plural(unassigned, "face")} nobody claimed</div>
            </div>
          </div>
        </Link>
      </ContextArea>
    )
  }

  return (
    <ContextArea menu={menu}>
      <Link
        to="/p/$pid/cast/$cid"
        params={{ pid, cid: "unassigned" }}
        className={
          compact
            ? "group flex aspect-[3/4] flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-white/15 text-center transition-colors hover:border-white/30 hover:bg-white/[0.03]"
            : "group flex aspect-[3/4] flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-white/15 text-center transition-colors hover:border-white/30 hover:bg-white/[0.03]"
        }
      >
        <span
          className={
            compact
              ? "flex size-10 items-center justify-center rounded-full bg-white/[0.06] text-fg-2 transition-colors group-hover:bg-white/[0.1] group-hover:text-fg-1 [&_svg]:size-5"
              : "flex size-14 items-center justify-center rounded-full bg-white/[0.06] text-fg-2 transition-colors group-hover:bg-white/[0.1] group-hover:text-fg-1 [&_svg]:size-6"
          }
        >
          <User />
        </span>
        <span className={compact ? "display text-lg text-fg-1" : "display text-2xl text-fg-1"}>Everyone else</span>
        <span className={compact ? "text-2xs text-fg-3" : "text-xs text-fg-3"}>{plural(unassigned, "face")}</span>
      </Link>
    </ContextArea>
  )
}
