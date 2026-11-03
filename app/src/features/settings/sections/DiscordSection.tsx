import { ArrowCounterClockwise } from "@/components/icons"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { DiscordIcon } from "@/components/DiscordIcon"
import { toast } from "sonner"
import type { AppSettings, DiscordRpcSettings } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Field, Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Tooltip } from "@/components/ui/tooltip"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient, useSettings } from "@/lib/queries"
import { SettingRow, SettingsGroup } from "../primitives"

const DEFAULT_DISCORD_RPC: Required<DiscordRpcSettings> = {
  enabled: true,
  clientId: "1567208524940058695",
  showTimer: true,
  showButtons: true,
  showProject: true,
  showEpisode: true,
  showCharacter: true,
  showProgress: true,
  showCounts: true,
  templateProject: "{project} • {episode}",
  templateScenepack: "Scenepack: {character} ({count} clips)",
  templateExport: "Exporting {item} • {progress}%",
}

async function saveSettings(next: AppSettings) {
  try {
    const s = await commands.settingsUpdate(next)
    queryClient.setQueryData(qk.settings, s)
  } catch (e) {
    toast.error("Couldn't save Discord settings", { description: errorMessage(e) })
  }
}

export function DiscordSection() {
  const { data: settings } = useSettings()
  const rpc = { ...DEFAULT_DISCORD_RPC, ...settings?.discordRpc }

  const updateRpc = (patch: Partial<DiscordRpcSettings>) => {
    if (!settings) return
    const nextRpc: DiscordRpcSettings = { ...rpc, ...patch }
    void saveSettings({ ...settings, discordRpc: nextRpc })
  }

  const resetDefaults = () => {
    if (!settings) return
    void saveSettings({ ...settings, discordRpc: DEFAULT_DISCORD_RPC })
    toast.success("Discord RPC settings reset to defaults")
  }

  return (
    <>
      {/* Live Preview Card */}
      <section className="mb-10">
        <div className="mb-3 px-1">
          <h3 className="text-lg font-semibold text-fg-1">Status Preview</h3>
          <p className="mt-0.5 text-md text-fg-2">How your activity appears to friends and servers on Discord.</p>
        </div>
        <div className="overflow-hidden rounded-2xl border border-white/[0.08] bg-[#111214] p-5 text-white shadow-xl">
          <div className="flex items-center gap-2 mb-4 text-xs font-semibold tracking-wider text-[#949ba4] uppercase">
            <DiscordIcon className="size-4 text-[#5865F2]" />
            Playing a Game
          </div>
          <div className="flex items-start gap-4">
            <div className="relative shrink-0">
              <div className="size-20 overflow-hidden rounded-2xl border border-white/[0.08] shadow-inner">
                <AutoSceneMark className="size-full" />
              </div>
              <Tooltip label="AutoScene">
                <div
                  className="absolute -right-1 -bottom-1 flex size-7 items-center justify-center rounded-full bg-[#2b2d31] border-2 border-[#111214] text-[#5865F2]"
                >
                  <DiscordIcon className="size-4" />
                </div>
              </Tooltip>
            </div>
            <div className="min-w-0 flex-1">
              <div className="truncate text-base font-bold text-white">AutoScene</div>
              <div className="truncate text-sm text-[#dbdee1]">
                {rpc.showProject ? "Tears of Steel • Episode 01" : "Editing project"}
              </div>
              <div className="truncate text-sm text-[#949ba4]">
                {rpc.showCharacter ? "Scenepack: Celia (48 clips)" : "Working on scenes"}
              </div>
              {rpc.showTimer && (
                <div className="mt-0.5 text-xs text-[#949ba4]">01:42:18 elapsed</div>
              )}
            </div>
          </div>
          {rpc.showButtons && (
            <div className="mt-4 flex flex-col gap-2 pt-2 border-t border-white/[0.06]">
              <div className="flex h-8 items-center justify-center rounded bg-[#2b2d31] px-4 text-xs font-medium text-white transition hover:bg-[#35373c] cursor-pointer">
                Get AutoScene
              </div>
            </div>
          )}
        </div>
      </section>

      {/* Main Connection Settings */}
      <SettingsGroup title="Rich Presence" description="Broadcast current project and editing activity to your Discord profile.">
        <SettingRow
          label="Enable Discord Rich Presence"
          description="Display rich activity status in Discord while AutoScene is running."
        >
          <Switch checked={rpc.enabled} onCheckedChange={(enabled) => updateRpc({ enabled })} />
        </SettingRow>

        <SettingRow
          label="Elapsed Timer"
          description="Show time elapsed since you started working in the app."
        >
          <Switch
            checked={rpc.showTimer}
            disabled={!rpc.enabled}
            onCheckedChange={(showTimer) => updateRpc({ showTimer })}
          />
        </SettingRow>

        <SettingRow
          label="Action Buttons"
          description="Show button links to the AutoScene project in your Discord profile."
        >
          <Switch
            checked={rpc.showButtons}
            disabled={!rpc.enabled}
            onCheckedChange={(showButtons) => updateRpc({ showButtons })}
          />
        </SettingRow>
      </SettingsGroup>

      {/* Privacy & Details Display */}
      <SettingsGroup title="Details & Privacy" description="Choose which editing details are visible in your status.">
        <SettingRow
          label="Project Name"
          description="Show the name of the project you are currently working on."
        >
          <Switch
            checked={rpc.showProject}
            disabled={!rpc.enabled}
            onCheckedChange={(showProject) => updateRpc({ showProject })}
          />
        </SettingRow>

        <SettingRow
          label="Episode / Source"
          description="Show active media episode or source video filename."
        >
          <Switch
            checked={rpc.showEpisode}
            disabled={!rpc.enabled}
            onCheckedChange={(showEpisode) => updateRpc({ showEpisode })}
          />
        </SettingRow>

        <SettingRow
          label="Character Name"
          description="Display the active character face/cast scenepack you are viewing."
        >
          <Switch
            checked={rpc.showCharacter}
            disabled={!rpc.enabled}
            onCheckedChange={(showCharacter) => updateRpc({ showCharacter })}
          />
        </SettingRow>

        <SettingRow
          label="Clip Counts"
          description="Display detected scene and clip counts."
        >
          <Switch
            checked={rpc.showCounts}
            disabled={!rpc.enabled}
            onCheckedChange={(showCounts) => updateRpc({ showCounts })}
          />
        </SettingRow>

        <SettingRow
          label="Export Progress"
          description="Broadcast real-time export encoding percentage while rendering."
        >
          <Switch
            checked={rpc.showProgress}
            disabled={!rpc.enabled}
            onCheckedChange={(showProgress) => updateRpc({ showProgress })}
          />
        </SettingRow>
      </SettingsGroup>

      {/* Custom Format Templates */}
      <SettingsGroup title="Status Templates" description="Customize presence text lines. Supported variables: {project}, {episode}, {character}, {count}, {item}, {progress}.">
        <div className="p-4 space-y-4">
          <Field label="Project Status Format" hint="Default: {project} • {episode}">
            <Input
              value={rpc.templateProject}
              disabled={!rpc.enabled}
              onChange={(e) => updateRpc({ templateProject: e.target.value })}
              placeholder="{project} • {episode}"
            />
          </Field>

          <Field label="Scenepack Details Format" hint="Default: Scenepack: {character} ({count} clips)">
            <Input
              value={rpc.templateScenepack}
              disabled={!rpc.enabled}
              onChange={(e) => updateRpc({ templateScenepack: e.target.value })}
              placeholder="Scenepack: {character} ({count} clips)"
            />
          </Field>

          <Field label="Export Activity Format" hint="Default: Exporting {item} • {progress}%">
            <Input
              value={rpc.templateExport}
              disabled={!rpc.enabled}
              onChange={(e) => updateRpc({ templateExport: e.target.value })}
              placeholder="Exporting {item} • {progress}%"
            />
          </Field>
        </div>
      </SettingsGroup>

      {/* Advanced / Developer */}
      <SettingsGroup title="Advanced" description="Custom Discord developer application configuration.">
        <div className="p-4 space-y-4">
          <Field
            label="Discord Application Client ID"
            hint="Leave as default unless using your own Discord developer application."
          >
            <Input
              value={rpc.clientId}
              disabled={!rpc.enabled}
              onChange={(e) => updateRpc({ clientId: e.target.value })}
              placeholder="1567208524940058695"
            />
          </Field>
        </div>
        <SettingRow
          label="Reset to Defaults"
          description="Restore all Discord Rich Presence settings and templates to factory defaults."
        >
          <Button variant="ghost" size="sm" onClick={resetDefaults}>
            <ArrowCounterClockwise /> Reset
          </Button>
        </SettingRow>
      </SettingsGroup>
    </>
  )
}
