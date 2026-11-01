import { Keyboard } from "@/components/icons"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { Kbd } from "@/components/ui/kbd"
import { useAppActions } from "@/lib/actions"
import { PLAYER_SHORTCUTS, capGroups, useKeymap } from "@/lib/shortcuts"
import { Card } from "./Card"

/** The player's keys, from the shortcut registry and the keymap (what is listed is what works). */
export function KeysCard() {
  const { goSettings } = useAppActions()
  const km = useKeymap()
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
        {PLAYER_SHORTCUTS.map((s) => {
          const groups = capGroups(s, km)
          return (
            <li key={s.id} className="flex items-center justify-between gap-3">
              <span>{s.label}</span>
              <span className="flex shrink-0 items-center gap-2">
                {groups.length > 0 ? (
                  groups.map((g, i) => (g.length > 0 ? <Kbd key={i} keys={g} /> : <NoKey key={i} />))
                ) : (
                  <NoKey />
                )}
              </span>
            </li>
          )
        })}
      </ul>
    </Card>
  )
}

/** A key the person took away. */
function NoKey() {
  return <span className="text-xs text-fg-3">No key</span>
}
