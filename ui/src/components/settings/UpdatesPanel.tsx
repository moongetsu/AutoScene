import { RefreshCw } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { InfoRow, SettingRow, SettingsGroup } from "@/components/settings/SettingsPrimitives"
import { useUpdates } from "@/state/updates"
import { cn } from "@/lib/utils"

/**
 * Manual "Check for Updates", and the switch that governs the automatic one.
 *
 * On macOS this duplicates the menu item; on Windows and Linux, where there is
 * no menu bar, it is the only manual entry point.
 */
export function UpdatesPanel() {
  const { status, checking, lastCheckedAt, autoCheck, supported, setAutoCheck, check } =
    useUpdates()

  if (!supported) return null

  const version = status?.currentVersion
  const pending = status?.state === "available" ? status.version : null

  return (
    <SettingsGroup
      title="Updates"
      description="AutoScene checks for a new version when it starts, and asks before installing anything."
      aside={
        <Button
          variant="outline"
          size="sm"
          disabled={checking}
          onClick={() => void check(true)}
        >
          <RefreshCw className={cn("size-3.5", checking && "animate-spin")} />
          {checking ? "Checking…" : "Check for Updates"}
        </Button>
      }
    >
      <InfoRow
        label="Installed version"
        value={
          pending ? (
            <span className="flex items-center gap-2">
              {version ?? "—"}
              <Badge variant="default" className="text-[10px]">
                {pending} available
              </Badge>
            </span>
          ) : (
            (version ?? "—")
          )
        }
      />

      <InfoRow
        label="Last checked"
        value={lastCheckedAt ? new Date(lastCheckedAt).toLocaleString() : "Not yet"}
      />

      <SettingRow
        label="Check automatically"
        hint="Looks for a new version on launch, then every few hours. Turn this off to only check by hand."
        htmlFor="auto-check-updates"
      >
        <div className="flex justify-start sm:justify-end">
          <Switch
            id="auto-check-updates"
            checked={autoCheck}
            onCheckedChange={setAutoCheck}
          />
        </div>
      </SettingRow>
    </SettingsGroup>
  )
}
