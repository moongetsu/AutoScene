import * as React from "react"
import { ShowcaseNav, type ShowcaseSectionId } from "./ShowcaseNav"
import { ButtonsShowcase } from "./sections/ButtonsShowcase"
import { BadgesShowcase } from "./sections/BadgesShowcase"
import { InputsShowcase } from "./sections/InputsShowcase"
import { TogglesShowcase } from "./sections/TogglesShowcase"
import { IndicatorsShowcase } from "./sections/IndicatorsShowcase"
import { OverlaysShowcase } from "./sections/OverlaysShowcase"
import { MediaShowcase } from "./sections/MediaShowcase"
import { CardsShowcase } from "./sections/CardsShowcase"
import { ShellShowcase } from "./sections/ShellShowcase"
import { TokensShowcase } from "./sections/TokensShowcase"
import { ButtonPlayground } from "./playground/ButtonPlayground"
import { AnimationTarget } from "./recorder/AnimationTarget"

export function ShowcaseLayout() {
  const [active, setActive] = React.useState<ShowcaseSectionId>("buttons")

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-canvas text-fg-1">
      {/* Navigation Sidebar */}
      <ShowcaseNav activeSection={active} onSelect={setActive} />

      {/* Main Content Area */}
      <main className="flex-1 overflow-y-auto p-10 bg-canvas">
        <div className="max-w-5xl mx-auto pb-24">
          {active === "buttons" && <ButtonsShowcase />}
          {active === "badges" && <BadgesShowcase />}
          {active === "inputs" && <InputsShowcase />}
          {active === "toggles" && <TogglesShowcase />}
          {active === "indicators" && <IndicatorsShowcase />}
          {active === "overlays" && <OverlaysShowcase />}
          {active === "media" && <MediaShowcase />}
          {active === "cards" && <CardsShowcase />}
          {active === "shell" && <ShellShowcase />}
          {active === "tokens" && <TokensShowcase />}
          {active === "playground" && <ButtonPlayground />}
          {active === "record" && <AnimationTarget />}
        </div>
      </main>
    </div>
  )
}
