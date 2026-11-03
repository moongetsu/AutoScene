import { Play, DownloadSimple, Trash, CircleNotch, Plus } from "@/components/icons"
import { Button } from "@/components/ui/button"

const VARIANTS = ["primary", "accent", "secondary", "glass", "ghost", "danger"] as const
const SIZES = ["lg", "md", "sm"] as const
const ICON_SIZES = ["icon-lg", "icon", "icon-sm"] as const

export function ButtonsShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Buttons & Pills</h2>
        <p className="text-fg-2 mt-1">
          Glassmorphic pill buttons crafted with CVA, backdrop-blur, subtle inner specular borders, and spring press states.
        </p>
      </div>

      {/* 1. Variants Matrix */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Variants</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-4 items-center">
          <Button variant="primary">
            <Play weight="fill" /> Primary Action
          </Button>
          <Button variant="accent">
            <Plus weight="bold" /> Accent Gold
          </Button>
          <Button variant="secondary">
            <DownloadSimple /> Secondary Dark
          </Button>
          <Button variant="glass">
            <DownloadSimple /> Frosted Glass
          </Button>
          <Button variant="ghost">Ghost Neutral</Button>
          <Button variant="danger">
            <Trash /> Danger Red
          </Button>
        </div>
      </section>

      {/* 2. Sizes Matrix */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Sizes (Standard & Icon)</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-col gap-6">
          <div className="flex flex-wrap items-center gap-4">
            {SIZES.map((size) => (
              <Button key={size} variant="accent" size={size}>
                Size {size.toUpperCase()}
              </Button>
            ))}
          </div>
          <div className="flex flex-wrap items-center gap-4">
            {ICON_SIZES.map((size) => (
              <Button key={size} variant="secondary" size={size} aria-label={`Icon button ${size}`}>
                <Plus weight="bold" />
              </Button>
            ))}
          </div>
        </div>
      </section>

      {/* 3. States Matrix */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Interactive States</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-5 items-center">
          <div className="flex flex-col items-center gap-2">
            <Button variant="primary">Default</Button>
            <span className="text-xs text-fg-3">Normal</span>
          </div>
          <div className="flex flex-col items-center gap-2">
            <Button variant="primary" disabled>
              Disabled
            </Button>
            <span className="text-xs text-fg-3">Disabled</span>
          </div>
          <div className="flex flex-col items-center gap-2">
            <Button variant="primary" className="gap-2 pointer-events-none">
              <CircleNotch className="animate-spin size-4" /> Processing…
            </Button>
            <span className="text-xs text-fg-3">Loading State</span>
          </div>
          <div className="flex flex-col items-center gap-2">
            <Button variant="accent" className="scale-[0.97] border-accent bg-accent/[0.26]">
              Pressed
            </Button>
            <span className="text-xs text-fg-3">Active Press</span>
          </div>
        </div>
      </section>

      {/* 4. Complete Matrix Table */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Complete Matrix (Variant x Size)</h3>
        <div className="overflow-x-auto rounded-2xl border border-white/[0.08] bg-canvas">
          <table className="w-full text-left text-sm">
            <thead className="border-b border-white/[0.08] bg-white/[0.02] text-xs uppercase text-fg-3 font-mono">
              <tr>
                <th className="px-6 py-4">Variant</th>
                <th className="px-6 py-4">Large (lg)</th>
                <th className="px-6 py-4">Medium (md)</th>
                <th className="px-6 py-4">Small (sm)</th>
                <th className="px-6 py-4">Icon (md)</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/[0.06]">
              {VARIANTS.map((variant) => (
                <tr key={variant} className="hover:bg-white/[0.01]">
                  <td className="px-6 py-4 font-mono font-medium text-accent">{variant}</td>
                  <td className="px-6 py-4">
                    <Button variant={variant} size="lg">Action</Button>
                  </td>
                  <td className="px-6 py-4">
                    <Button variant={variant} size="md">Action</Button>
                  </td>
                  <td className="px-6 py-4">
                    <Button variant={variant} size="sm">Action</Button>
                  </td>
                  <td className="px-6 py-4">
                    <Button variant={variant} size="icon" aria-label="Icon">
                      <Play weight="fill" />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  )
}
