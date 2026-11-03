import { Badge } from "@/components/ui/badge"

export function BadgesShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Badges & Status Tags</h2>
        <p className="text-fg-2 mt-1">
          Pill tags and status indicators used across episodes, exports, and character profiles.
        </p>
      </div>

      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Tones</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-4 items-center">
          <Badge>Default Neutral</Badge>
          <Badge tone="accent">Accent Gold</Badge>
          <Badge tone="danger">Danger Red</Badge>
          <Badge tone="outline">Outline</Badge>
          <Badge tone="media">Media Frost</Badge>
        </div>
      </section>

      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Use Case Samples</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-6 items-center">
          <div className="flex items-center gap-2">
            <span className="text-sm text-fg-2">Export Status:</span>
            <Badge tone="accent">Done</Badge>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-fg-2">Activity:</span>
            <Badge tone="accent">Exporting</Badge>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-fg-2">Cut Verification:</span>
            <Badge tone="danger">3 Unverified</Badge>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-fg-2">Clip Counter:</span>
            <Badge>42 clips</Badge>
          </div>
        </div>
      </section>
    </div>
  )
}
