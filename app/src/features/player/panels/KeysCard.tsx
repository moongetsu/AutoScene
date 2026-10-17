import { Keyboard } from "@/components/icons"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { Kbd } from "@/components/ui/kbd"
import { useAppActions } from "@/lib/actions"
import { PLAYER_SHORTCUTS } from "@/lib/shortcuts"
import { Card } from "./Card"

/** The player's keys, from the shortcut registry (what is listed is what works). */
export function KeysCard() {
  const { goSettings } = useAppActions()
  return (
    <Card
      title="Keys"
      menu={
        <ContextMenuItem onSelect={() => goSettings("shortcuts")}>
          <Keyboard /> All keyboard shortcuts
        </ContextMenuItem>
      }
    >
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
