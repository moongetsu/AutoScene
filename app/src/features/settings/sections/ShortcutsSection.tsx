import { Kbd } from "@/components/ui/kbd"
import { SHORTCUTS } from "@/lib/shortcuts"
import { SettingRow, SettingsGroup } from "../primitives"

export function ShortcutsSection() {
  return (
    <SettingsGroup title="Keyboard shortcuts" description="Work faster without leaving the keyboard.">
      {SHORTCUTS.map((s) => (
        <SettingRow key={s.id} label={s.label} className="min-h-11 py-2">
          <Kbd keys={s.keys} />
        </SettingRow>
      ))}
    </SettingsGroup>
  )
}
