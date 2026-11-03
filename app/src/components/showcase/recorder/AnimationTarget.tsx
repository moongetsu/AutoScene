import * as React from "react"
import { Play } from "@phosphor-icons/react"
import { Button } from "@/components/ui/button"

export function AnimationTarget() {
  const [pressed, setPressed] = React.useState(false)

  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Animation Recorder Stage</h2>
        <p className="text-fg-2 mt-1">
          Isolated capture stage with a fixed `#animation-target` element for headless Playwright recording.
        </p>
      </div>

      <div className="rounded-2xl border border-white/[0.08] bg-canvas p-16 flex flex-col items-center justify-center min-h-[400px]">
        {/* Exact target container */}
        <div id="animation-target" className="p-12 rounded-3xl bg-panel border border-white/[0.08] shadow-2xl flex flex-col items-center gap-6">
          <span className="text-xs font-mono uppercase tracking-widest text-accent">Component Target</span>
          
          <Button
            id="recorder-button"
            variant="primary"
            size="lg"
            className="shadow-2xl"
            onMouseDown={() => setPressed(true)}
            onMouseUp={() => setPressed(false)}
          >
            <Play weight="fill" /> Build a scenepack
          </Button>

          <span className="text-xs text-fg-3">
            State: <span className="font-mono text-fg-1">{pressed ? "pressed" : "idle"}</span>
          </span>
        </div>
      </div>
    </div>
  )
}
