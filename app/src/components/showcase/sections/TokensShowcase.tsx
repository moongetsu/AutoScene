const SURFACES = [
  { name: "--bg-inset", hex: "#050506", label: "Inset / Well" },
  { name: "--bg-canvas", hex: "#09090b", label: "Canvas" },
  { name: "--bg-panel", hex: "#111114", label: "Panel / Card" },
  { name: "--bg-raised", hex: "#18181c", label: "Raised" },
  { name: "--bg-overlay", hex: "#202025", label: "Overlay / Popover" },
]

const ACCENTS = [
  { name: "--accent", hex: "#e0b45c", label: "Brand Accent Gold" },
  { name: "--accent-hover", hex: "#eac375", label: "Accent Hover" },
  { name: "--accent-press", hex: "#c99e48", label: "Accent Press" },
  { name: "--accent-subtle", hex: "rgba(224, 180, 92, 0.14)", label: "Accent Subtle Glow" },
]

const TEXTS = [
  { name: "--fg-1", hex: "#f5f3ef", label: "Primary Warm White" },
  { name: "--fg-2", hex: "#b4b1ac", label: "Secondary Muted" },
  { name: "--fg-3", hex: "#8a8783", label: "Tertiary Subdued" },
  { name: "--fg-disabled", hex: "#4f4d4a", label: "Disabled" },
]

export function TokensShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Design Tokens</h2>
        <p className="text-fg-2 mt-1">
          AutoScene "Cinematic" design tokens: near-black theatre surfaces, warm white typography, and gold accent.
        </p>
      </div>

      {/* Surfaces */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Surfaces</h3>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
          {SURFACES.map((s) => (
            <div key={s.name} className="rounded-2xl border border-white/[0.08] bg-canvas p-4 flex flex-col gap-3">
              <div
                className="h-16 w-full rounded-xl border border-white/[0.1] shadow-inner"
                style={{ backgroundColor: s.hex }}
              />
              <div>
                <span className="text-xs font-semibold text-fg-1 block">{s.label}</span>
                <span className="text-[11px] font-mono text-fg-3">{s.name}</span>
                <span className="text-[10px] font-mono text-accent block">{s.hex}</span>
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* Brand Accent */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Brand Accent</h3>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          {ACCENTS.map((a) => (
            <div key={a.name} className="rounded-2xl border border-white/[0.08] bg-canvas p-4 flex flex-col gap-3">
              <div
                className="h-16 w-full rounded-xl border border-white/[0.1] shadow-inner"
                style={{ backgroundColor: a.hex }}
              />
              <div>
                <span className="text-xs font-semibold text-fg-1 block">{a.label}</span>
                <span className="text-[11px] font-mono text-fg-3">{a.name}</span>
                <span className="text-[10px] font-mono text-accent block">{a.hex}</span>
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* Typography Hierarchy */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Typography Hierarchy</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-4 max-w-xl">
          {TEXTS.map((t) => (
            <div key={t.name} className="flex items-center justify-between border-b border-white/[0.06] pb-3">
              <div>
                <p className="text-sm font-medium" style={{ color: t.hex }}>
                  The quick brown fox jumps over the lazy dog
                </p>
                <span className="text-[11px] text-fg-3 font-mono">{t.name} ({t.label})</span>
              </div>
              <span className="text-xs font-mono text-fg-3">{t.hex}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}
