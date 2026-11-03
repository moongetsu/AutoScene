import { useState } from "react"
import { Switch } from "@/components/ui/switch"
import { Slider } from "@/components/ui/slider"
import { Segmented } from "@/components/ui/segmented"

export function TogglesShowcase() {
  const [toggleState, setToggleState] = useState(true)
  const [sliderVal, setSliderVal] = useState(75)
  const [tab, setTab] = useState<"episodes" | "cast" | "packs">("cast")

  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Toggles, Sliders & Segmented Controls</h2>
        <p className="text-fg-2 mt-1">
          Smooth interactive controls with spring transitions and precision dragging.
        </p>
      </div>

      {/* Segmented */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Segmented Sliding Tabs</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-4 max-w-md">
          <Segmented
            aria-label="View tabs"
            value={tab}
            onChange={(v) => setTab(v)}
            options={[
              { value: "episodes", label: "Episodes" },
              { value: "cast", label: "Cast" },
              { value: "packs", label: "Scenepacks" },
            ]}
          />
          <span className="text-xs text-fg-3">Selected tab: <span className="text-accent font-mono">{tab}</span></span>
        </div>
      </section>

      {/* Switches */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Switches</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-4 max-w-md">
          <label className="flex items-center justify-between gap-4 cursor-pointer">
            <span className="text-sm font-medium text-fg-1">Hardware Acceleration</span>
            <Switch checked={toggleState} onCheckedChange={setToggleState} />
          </label>
          <label className="flex items-center justify-between gap-4 cursor-pointer">
            <span className="text-sm font-medium text-fg-1">Auto-verify every cut</span>
            <Switch checked={false} onCheckedChange={() => {}} />
          </label>
        </div>
      </section>

      {/* Sliders */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Sliders & Range Controls</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-4 max-w-md">
          <div className="flex justify-between text-xs text-fg-2">
            <span>Detection Confidence Threshold</span>
            <span className="font-mono text-accent">{sliderVal}%</span>
          </div>
          <Slider
            label="Confidence Threshold"
            value={sliderVal}
            max={100}
            min={0}
            step={1}
            onChange={(val) => setSliderVal(val)}
          />
        </div>
      </section>
    </div>
  )
}
