import type { SettingsSection } from "@/lib/actions"

export const SETTINGS_LABELS: Record<SettingsSection, string> = {
  general: "General",
  shortcuts: "Shortcuts",
  updates: "Updates",
  about: "About",
}

export function settingsLabel(section: string): string {
  return SETTINGS_LABELS[section as SettingsSection] ?? "Settings"
}
