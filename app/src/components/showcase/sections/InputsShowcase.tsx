import * as React from "react"
import { MagnifyingGlass } from "@phosphor-icons/react"
import { Input } from "@/components/ui/input"

export function InputsShowcase() {
  const [val, setVal] = React.useState("The Mentalist")

  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Inputs & Search</h2>
        <p className="text-fg-2 mt-1">
          Dark glass form inputs with subtle borders and clear/focus feedback.
        </p>
      </div>

      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Standard Text Inputs</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 grid grid-cols-1 md:grid-cols-2 gap-6 max-w-2xl">
          <div className="flex flex-col gap-2">
            <label className="text-xs font-medium text-fg-2">Project Name</label>
            <Input value={val} onChange={(e) => setVal(e.target.value)} placeholder="Enter project name..." />
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs font-medium text-fg-2">Empty with Placeholder</label>
            <Input placeholder="Type something here…" />
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs font-medium text-fg-2">Disabled Input</label>
            <Input disabled value="Read-only system path" />
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs font-medium text-fg-2">Search Input with Icon</label>
            <div className="relative">
              <MagnifyingGlass className="absolute left-3.5 top-1/2 -translate-y-1/2 size-4 text-fg-3" />
              <Input className="pl-10" placeholder="Search cast or episodes…" />
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
