import { MagnifyingGlass, GearSix, Play, Clock } from "@/components/icons"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"

export function ShellShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Shell & System</h2>
        <p className="text-fg-2 mt-1">
          Window chrome, navigation rail icons, brand logos, and shell surfaces.
        </p>
      </div>

      {/* Brand Logos */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Brand & Vector Marks</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-8 items-center">
          <div className="flex flex-col items-center gap-2">
            <AutoSceneMark className="size-10" />
            <span className="text-xs text-fg-3">AutoScene Mark</span>
          </div>
        </div>
      </section>

      {/* Navigation Rail Preview */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Navigation Rail Component</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex items-center">
          <div className="w-16 rounded-2xl border border-white/[0.08] bg-panel py-6 flex flex-col items-center gap-6 shadow-2xl">
            <AutoSceneMark className="size-7" />

            <div className="flex flex-col items-center gap-3 w-full">
              <button
                type="button"
                className="size-10 rounded-xl bg-accent/15 text-accent flex items-center justify-center transition-all"
                aria-label="Library"
              >
                <Play weight="fill" className="size-5" />
              </button>
              <button
                type="button"
                className="size-10 rounded-xl text-fg-3 hover:bg-white/[0.04] hover:text-fg-1 flex items-center justify-center transition-all"
                aria-label="Activity"
              >
                <Clock className="size-5" />
              </button>
              <button
                type="button"
                className="size-10 rounded-xl text-fg-3 hover:bg-white/[0.04] hover:text-fg-1 flex items-center justify-center transition-all"
                aria-label="Settings"
              >
                <GearSix className="size-5" />
              </button>
            </div>
          </div>
        </div>
      </section>

      {/* Command Palette Mock */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Command Palette Surface</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 max-w-lg">
          <div className="rounded-2xl border border-white/[0.12] bg-panel shadow-2xl overflow-hidden">
            <div className="flex items-center gap-3 px-4 py-3.5 border-b border-white/[0.08]">
              <MagnifyingGlass className="size-4 text-fg-3" />
              <input
                disabled
                placeholder="Type a command or search…"
                className="bg-transparent text-sm text-fg-1 placeholder:text-fg-3 focus:outline-none w-full"
              />
              <span className="text-[10px] font-mono bg-white/[0.06] text-fg-3 px-1.5 py-0.5 rounded">ESC</span>
            </div>
            <div className="p-2 flex flex-col gap-1 text-sm">
              <div className="px-3 py-2 rounded-xl bg-accent/15 text-accent flex items-center justify-between">
                <span>Filter: Speech & Vocals Only</span>
                <span className="text-xs font-mono text-fg-3">Plugin</span>
              </div>
              <div className="px-3 py-2 rounded-xl text-fg-2 hover:bg-white/[0.04] flex items-center justify-between">
                <span>Export all Patrick Jane scenes</span>
                <span className="text-xs font-mono text-fg-3">⌘E</span>
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
