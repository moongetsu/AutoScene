import { FolderOpen } from "@/components/icons"
import { toast } from "sonner"
import type { AppSettings } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Select } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { commands, errorMessage } from "@/lib/ipc"
import { PREFERRED_AUDIO_LANGUAGES, pickFolder } from "@/lib/media"
import { qk, queryClient, useAppInfo, usePresets, useSettings } from "@/lib/queries"
import { PathText, SettingRow, SettingsGroup } from "../primitives"

function reveal(path: string) {
  commands.revealPath(path).catch((e) => toast.error("Couldn't open folder", { description: errorMessage(e) }))
}

async function save(next: AppSettings) {
  try {
    const s = await commands.settingsUpdate(next)
    queryClient.setQueryData(qk.settings, s)
    void queryClient.invalidateQueries({ queryKey: qk.appInfo })
    void queryClient.invalidateQueries({ queryKey: ["exportDir"] })
  } catch (e) {
    toast.error("Couldn't save the setting", { description: errorMessage(e) })
  }
}

export function GeneralSection() {
  const { data: info } = useAppInfo()
  const { data: settings } = useSettings()
  const { data: presets } = usePresets()

  const changeLibrary = async () => {
    if (!settings) return
    const dir = await pickFolder("Choose the library folder")
    if (dir) await save({ ...settings, libraryRoot: dir })
  }

  return (
    <>
      <SettingsGroup title="Storage" description="Where AutoScene keeps projects, models and logs.">
        <SettingRow
          label="Library location"
          description={
            settings && (
              <>
                <PathText path={settings.libraryRoot} />
                <span className="mt-1 block text-fg-3">New projects and default exports go here. Existing projects stay where they are.</span>
              </>
            )
          }
        >
          <Button size="sm" variant="ghost" disabled={!settings} onClick={() => settings && reveal(settings.libraryRoot)}>
            <FolderOpen /> Reveal
          </Button>
          <Button size="sm" disabled={!settings} onClick={() => void changeLibrary()}>
            Change…
          </Button>
        </SettingRow>
        <SettingRow label="App data" description={info && <PathText path={info.dataDir} />}>
          <Button size="sm" variant="ghost" disabled={!info} onClick={() => info && reveal(info.dataDir)}>
            <FolderOpen /> Reveal
          </Button>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Characters" description="How the analysis finds who is in a shot.">
        <SettingRow
          label="Find characters without a visible face"
          description="Also follows people by their body and clothes, so shots where someone faces away or is too small for their face join their scenepack, marked “No face”. Analysis takes about a quarter longer; episodes analysed before are looked through again for their people."
        >
          <Switch
            aria-label="Find characters without a visible face"
            checked={settings?.findBodies ?? false}
            disabled={!settings}
            onCheckedChange={(on) => settings && void save({ ...settings, findBodies: on })}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Audio" description="Many files carry several soundtracks, and dubs are often marked as the default.">
        <SettingRow
          label="Preferred language"
          description="Chosen automatically when an episode is added. You can switch the track per episode at any time."
        >
          <Select
            aria-label="Preferred audio language"
            className="w-[260px]"
            value={settings && (settings.audioLanguage || "default")}
            disabled={!settings}
            onChange={(lang) => settings && void save({ ...settings, audioLanguage: lang === "default" ? "" : lang })}
            options={PREFERRED_AUDIO_LANGUAGES.map((l) => ({ ...l, value: l.value || "default" }))}
          />
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Export" description="Defaults for new exports. You can change them per export.">
        <SettingRow label="Format" description={presets?.find((p) => p.id === settings?.exportPresetId)?.description}>
          <Select
            aria-label="Default export format"
            className="w-[260px]"
            value={settings?.exportPresetId}
            disabled={!settings}
            onChange={(id) => settings && void save({ ...settings, exportPresetId: id })}
            options={(presets ?? []).map((p) => ({ value: p.id, label: p.name }))}
          />
        </SettingRow>
      </SettingsGroup>
    </>
  )
}
