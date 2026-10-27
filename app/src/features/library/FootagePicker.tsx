import { useEffect } from "react"
import type { ContentKind } from "@/bindings"
import { Segmented } from "@/components/ui/segmented"
import { FOOTAGE, type Footage } from "@/lib/footage"

/**
 * What the footage is: live action, 3D animation, anime, mixed. Resting on
 * an option shows example stills of it. `locked` names the options that
 * can't be chosen, with why (shown in their preview).
 */
export function FootagePicker({
  value,
  onChange,
  locked,
}: {
  value: ContentKind
  onChange: (v: ContentKind) => void
  locked?: Partial<Record<ContentKind, string>>
}) {
  // Fetched ahead, so a preview shows whole the moment it opens.
  useEffect(() => {
    for (const f of FOOTAGE) for (const src of f.stills) new Image().src = src
  }, [])
  return (
    <Segmented<ContentKind>
      aria-label="Footage"
      value={value}
      onChange={onChange}
      options={FOOTAGE.map((f) => ({
        value: f.value,
        label: f.label,
        disabled: locked?.[f.value] != null,
        preview: <FootagePreview footage={f} locked={locked?.[f.value]} />,
      }))}
    />
  )
}

function FootagePreview({ footage: f, locked }: { footage: Footage; locked?: string }) {
  const [wide, a, b] = f.stills
  return (
    <div>
      <img src={wide} alt="" className="aspect-[672/281] w-full rounded-xl bg-white/[0.04] object-cover" />
      <div className="mt-1.5 grid grid-cols-2 gap-1.5">
        <img src={a} alt="" className="aspect-video w-full rounded-lg bg-white/[0.04] object-cover" />
        <img src={b} alt="" className="aspect-video w-full rounded-lg bg-white/[0.04] object-cover" />
      </div>
      <div className="mt-3 px-1 pb-0.5">
        <div className="text-sm font-medium text-fg-1">{f.label}</div>
        <p className="mt-0.5 text-xs text-fg-2">{f.blurb}</p>
        {locked && <p className="mt-1.5 text-xs text-accent">{locked}</p>}
        <p className="mt-2 text-2xs text-fg-3">Stills: {f.credit}</p>
      </div>
    </div>
  )
}
