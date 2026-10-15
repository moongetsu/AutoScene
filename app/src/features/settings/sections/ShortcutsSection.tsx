import { Kbd } from "@/components/ui/kbd"
import { SHORTCUT_GROUPS } from "@/lib/shortcuts"
import { SettingRow, SettingsGroup } from "../primitives"

export function ShortcutsSection() {
  return (
    <>
      {SHORTCUT_GROUPS.map((g) => (
        <SettingsGroup key={g.title} title={g.title} description={g.description}>
          {g.shortcuts.map((s) => (
            <SettingRow key={s.id} label={s.label} className="min-h-11 py-2">
              <Kbd keys={s.keys} />
            </SettingRow>
          ))}
        </SettingsGroup>
      ))}
    </>
  )
}
