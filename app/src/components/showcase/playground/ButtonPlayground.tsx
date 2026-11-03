import { useState } from "react"
import { Play } from "@phosphor-icons/react"
import { Slider } from "@/components/ui/slider"
import { Switch } from "@/components/ui/switch"

export function ButtonPlayground() {
  const [radius, setRadius] = useState(9999) // default pill (9999px)
  const [innerGlow, setInnerGlow] = useState(30) // 30%
  const [scaleOnPress, setScaleOnPress] = useState(true)
  const [blurLevel, setBlurLevel] = useState(24) // 24px

  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Live Button Playground</h2>
        <p className="text-fg-2 mt-1">
          Adjust visual parameters in real time to craft and preview custom button aesthetics before applying them globally.
        </p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-8 items-start">
        {/* Controls */}
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-6">
          <h3 className="text-sm font-semibold uppercase tracking-wider text-fg-3 font-mono">Modifiers</h3>

          {/* Corner Radius */}
          <div className="flex flex-col gap-2">
            <div className="flex justify-between text-xs text-fg-2">
              <span>Corner Radius</span>
              <span className="font-mono text-accent">{radius > 100 ? "Full Pill" : `${radius}px`}</span>
            </div>
            <Slider
              label="Corner Radius"
              value={radius > 100 ? 50 : radius}
              max={50}
              min={4}
              step={2}
              onChange={(val) => setRadius(val === 50 ? 9999 : val)}
            />
          </div>

          {/* Inner Specular Border Glow */}
          <div className="flex flex-col gap-2">
            <div className="flex justify-between text-xs text-fg-2">
              <span>Inner Specular Glow Opacity</span>
              <span className="font-mono text-accent">{innerGlow}%</span>
            </div>
            <Slider
              label="Inner Specular Glow"
              value={innerGlow}
              max={100}
              min={0}
              step={5}
              onChange={(val) => setInnerGlow(val)}
            />
          </div>

          {/* Backdrop Blur */}
          <div className="flex flex-col gap-2">
            <div className="flex justify-between text-xs text-fg-2">
              <span>Backdrop Blur Intensity</span>
              <span className="font-mono text-accent">{blurLevel}px</span>
            </div>
            <Slider
              label="Backdrop Blur"
              value={blurLevel}
              max={40}
              min={0}
              step={2}
              onChange={(val) => setBlurLevel(val)}
            />
          </div>

          {/* Spring Press Feedback */}
          <label className="flex items-center justify-between gap-4 cursor-pointer pt-2 border-t border-white/[0.06]">
            <span className="text-sm font-medium text-fg-1">Spring Scale on Active Press</span>
            <Switch checked={scaleOnPress} onCheckedChange={setScaleOnPress} />
          </label>
        </div>

        {/* Live Preview Box */}
        <div className="rounded-2xl border border-white/[0.08] bg-panel p-10 flex flex-col items-center justify-center min-h-[360px] gap-8 relative overflow-hidden">
          <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,_var(--tw-gradient-stops))] from-accent/10 via-transparent to-transparent pointer-events-none" />

          <span className="text-xs uppercase font-mono tracking-widest text-fg-3">Interactive Target</span>

          <button
            type="button"
            className="inline-flex select-none items-center justify-center gap-2 whitespace-nowrap font-semibold text-white px-6 h-11 text-md transition-all duration-150 ease-out hover:brightness-110 active:brightness-95"
            style={{
              borderRadius: radius > 100 ? "9999px" : `${radius}px`,
              backgroundColor: "rgba(255, 255, 255, 0.18)",
              backdropFilter: `blur(${blurLevel}px)`,
              border: "1px solid rgba(255, 255, 255, 0.4)",
              boxShadow: `inset 0 1px 0 rgba(255, 255, 255, ${innerGlow / 100})`,
              transform: scaleOnPress ? undefined : "none",
            }}
          >
            <Play weight="fill" className="size-4" /> Customized Action
          </button>

          <div className="text-center">
            <p className="text-xs text-fg-3">Click or tap to inspect the spring physics and specular reflection.</p>
          </div>
        </div>
      </div>
    </div>
  )
}
