import { Kbd } from "@/components/ui/kbd"
import { PLAYER_SHORTCUTS } from "@/lib/shortcuts"
import { Card } from "./Card"

/** The player's keys, from the shortcut registry (what is listed is what works). */
export function KeysCard() {
  return (
    <Card title="Keys">
      <ul className="space-y-2 text-sm text-fg-2">
        {PLAYER_SHORTCUTS.map((s) => (
          <li key={s.id} className="flex items-center justify-between gap-3">
            <span>{s.label}</span>
            <Kbd keys={s.keys} className="shrink-0" />
          </li>
        ))}
      </ul>
    </Card>
  )
}
