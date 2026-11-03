import { Play, DotsThree, FilmStrip } from "@/components/icons"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

export function CardsShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Composite Cards</h2>
        <p className="text-fg-2 mt-1">
          Production UI cards used across Cast, Projects, Episodes, and Scenepacks.
        </p>
      </div>

      {/* Character Card Preview */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Character Profile Card</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-6 items-center">
          <div className="group relative w-64 rounded-2xl border border-white/[0.08] bg-panel p-4 flex flex-col gap-3 transition-all hover:border-white/20 hover:shadow-xl">
            <div className="relative aspect-square w-full rounded-xl overflow-hidden bg-overlay">
              <img
                src="https://images.unsplash.com/photo-1507003211169-0a1dd7228f2d?w=400&fit=crop"
                alt="Patrick Jane"
                className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
              />
              <div className="absolute top-2.5 right-2.5">
                <Badge tone="accent">48 clips</Badge>
              </div>
            </div>
            <div className="flex items-center justify-between">
              <div>
                <h4 className="font-semibold text-fg-1">Patrick Jane</h4>
                <p className="text-xs text-fg-3">The Mentalist</p>
              </div>
              <Button variant="ghost" size="icon-sm" aria-label="More options">
                <DotsThree weight="bold" />
              </Button>
            </div>
          </div>
        </div>
      </section>

      {/* Episode / Export Row Card Preview */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Export Summary Card</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 max-w-2xl">
          <div className="rounded-2xl border border-white/[0.08] bg-panel p-5 flex items-center justify-between gap-4">
            <div className="flex items-center gap-4">
              <div className="flex size-11 items-center justify-center rounded-xl bg-accent-subtle text-accent">
                <FilmStrip className="size-5" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="font-semibold text-fg-1">Mentalist_Patrick_Jane_scenepack.mov</h4>
                  <Badge tone="accent">Verified</Badge>
                </div>
                <p className="text-xs text-fg-3 mt-0.5">ProRes 422 · 42 clips · 2m 14s · 108.2 MB</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="secondary" size="sm">
                <Play weight="fill" /> Play
              </Button>
              <Button variant="ghost" size="icon-sm" aria-label="Export options">
                <DotsThree weight="bold" />
              </Button>
            </div>
          </div>
        </div>
      </section>

      {/* Shot Card Preview */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Shot Detection Cut Card</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-4">
          {[1, 2, 3].map((shot) => (
            <div
              key={shot}
              className="w-52 rounded-xl border border-white/[0.08] bg-panel overflow-hidden group hover:border-white/20 transition-all cursor-pointer"
            >
              <div className="aspect-video bg-overlay relative flex items-center justify-center">
                <FilmStrip className="size-6 text-fg-3" />
                <span className="absolute bottom-1.5 right-1.5 text-[10px] font-mono bg-black/60 px-1.5 py-0.5 rounded text-white backdrop-blur">
                  00:0{shot}:14
                </span>
              </div>
              <div className="p-2.5 flex justify-between items-center text-xs">
                <span className="font-medium text-fg-1">Cut #{shot}</span>
                <span className="text-fg-3 font-mono">72 frames</span>
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}
