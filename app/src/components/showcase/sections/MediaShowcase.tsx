import { Hero } from "@/components/ui/hero"
import { Badge } from "@/components/ui/badge"

export function MediaShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Media & Artwork</h2>
        <p className="text-fg-2 mt-1">
          Artwork image frames with smooth placeholder loaders and cinematic hero banners.
        </p>
      </div>

      {/* Hero */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Hero Display Header</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas overflow-hidden">
          <Hero path={null}>
            <div className="p-8 flex flex-col gap-2">
              <h1 className="text-4xl font-bold text-fg-1">The Mentalist</h1>
              <div className="flex items-center gap-2">
                <Badge tone="accent">Season 1</Badge>
                <span className="text-sm text-fg-3">23 Episodes · 1,420 Cast Faces</span>
              </div>
            </div>
          </Hero>
        </div>
      </section>

      {/* Artwork Containers */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Artwork Cards</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-6 items-center">
          <div className="w-48 flex flex-col gap-2">
            <span className="text-xs text-fg-3 font-medium">Portrait Aspect (Poster)</span>
            <div className="h-64 rounded-xl overflow-hidden border border-white/[0.08] bg-panel">
              <img
                src="https://images.unsplash.com/photo-1534447677768-be436bb09401?w=500"
                alt="Poster sample"
                className="size-full object-cover"
              />
            </div>
          </div>

          <div className="w-64 flex flex-col gap-2">
            <span className="text-xs text-fg-3 font-medium">16:9 Video Still</span>
            <div className="aspect-video rounded-xl overflow-hidden border border-white/[0.08] bg-panel">
              <img
                src="https://images.unsplash.com/photo-1574375927938-d5a98e8ffe85?w=500"
                alt="Video still sample"
                className="size-full object-cover"
              />
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
