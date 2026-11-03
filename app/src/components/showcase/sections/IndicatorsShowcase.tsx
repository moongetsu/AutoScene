import { Plus } from "@/components/icons"
import { Progress } from "@/components/ui/progress"
import { EmptyState } from "@/components/ui/empty-state"
import { ErrorState } from "@/components/ui/error-state"
import { Kbd } from "@/components/ui/kbd"

export function IndicatorsShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Indicators & States</h2>
        <p className="text-fg-2 mt-1">
          Progress bars, shortcut keys (Kbd), and placeholder states.
        </p>
      </div>

      {/* Progress */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Progress Indicators</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-6 max-w-lg">
          <div className="flex flex-col gap-2">
            <div className="flex justify-between text-xs text-fg-2">
              <span>Exporting scene clips…</span>
              <span className="font-mono text-accent">65%</span>
            </div>
            <Progress value={65} />
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex justify-between text-xs text-fg-2">
              <span>Indeterminate Loading</span>
            </div>
            <Progress />
          </div>
        </div>
      </section>

      {/* Keyboard Shortcuts */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Keyboard Shortcuts (Kbd)</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-4 items-center">
          <div className="flex items-center gap-2 text-sm text-fg-2">
            <span>Command Palette:</span>
            <Kbd keys={["⌘", "K"]} />
          </div>
          <div className="flex items-center gap-2 text-sm text-fg-2">
            <span>Play/Pause:</span>
            <Kbd keys={["Space"]} />
          </div>
          <div className="flex items-center gap-2 text-sm text-fg-2">
            <span>Mark In:</span>
            <Kbd keys={["I"]} />
          </div>
          <div className="flex items-center gap-2 text-sm text-fg-2">
            <span>Mark Out:</span>
            <Kbd keys={["O"]} />
          </div>
        </div>
      </section>

      {/* Empty & Error States */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Empty & Error States</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6">
            <EmptyState
              icon={<Plus className="size-8 text-accent" />}
              title="No faces detected"
              description="Drop an episode into the project to let AutoScene find and group characters."
            />
          </div>
          <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6">
            <ErrorState
              title="Failed to open episode"
              error={new Error("The file at this path could not be decoded.")}
            />
          </div>
        </div>
      </section>
    </div>
  )
}
