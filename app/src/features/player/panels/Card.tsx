import type * as React from "react"
import { ContextArea, type MenuContent } from "@/components/ui/context-area"

/** A titled card of the details panel; `menu`: what a right-click on it offers. */
export function Card({
  title,
  action,
  menu,
  children,
}: {
  title: string
  action?: React.ReactNode
  menu?: MenuContent
  children: React.ReactNode
}) {
  return (
    <ContextArea menu={menu ?? null} disabled={menu === undefined}>
      <section className="rounded-2xl border border-white/[0.06] bg-white/[0.03] p-4">
        <div className="mb-3 flex min-h-7 items-center justify-between gap-2">
          <h2 className="text-sm font-semibold text-fg-1">{title}</h2>
          {action}
        </div>
        {children}
      </section>
    </ContextArea>
  )
}
