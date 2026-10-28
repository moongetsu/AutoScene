import type { AppSettings } from "@/bindings"
import { commands } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"

/**
 * The format an export started with becomes the next one's. The export is
 * on its way already: when this can't be saved, the next dialog only opens
 * on the old format, so it is noted in the log rather than shown.
 */
export function rememberPreset(settings: AppSettings | undefined, presetId: string) {
  if (!settings || settings.exportPresetId === presetId) return
  commands
    .settingsUpdate({ ...settings, exportPresetId: presetId })
    .then((s) => queryClient.setQueryData(qk.settings, s))
    .catch((e) => console.warn("remembering the export format:", e))
}
