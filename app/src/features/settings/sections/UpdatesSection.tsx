import type * as React from "react"
import { CheckCircle2, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { useAppInfo } from "@/lib/queries"
import { useUpdates } from "@/state/updates"
import { SettingRow, SettingsGroup } from "../primitives"

function relativeTime(ms: number): string {
  const s = Math.round((Date.now() - ms) / 1000)
  if (s < 60) return "just now"
  const m = Math.round(s / 60)
  if (m < 60) return `${m} min ago`
  return `${Math.round(m / 60)} h ago`
}

export function UpdatesSection() {
  const { data: info } = useAppInfo()
  const u = useUpdates()
  const checking = u.phase === "checking"

  let statusLine: React.ReactNode = "Checks automatically at launch and every few hours."
  if (checking) statusLine = "Checking…"
  else if (u.phase === "current")
    statusLine = (
      <span className="inline-flex items-center gap-1.5">
        <CheckCircle2 className="size-3.5 text-fg-2" /> You're up to date
        {u.lastChecked && <span className="text-fg-3"> · checked {relativeTime(u.lastChecked)}</span>}
      </span>
    )
  else if (u.phase === "error") statusLine = <span className="text-danger">{u.error}</span>
  else if (u.status?.state === "available") statusLine = `Version ${u.status.version} is ready to install.`

  return (
    <SettingsGroup title="Updates" description="AutoScene updates itself in place. Your projects and models are kept.">
      <SettingRow
        label={
          <span className="flex items-center gap-2">
            AutoScene {info?.version}
            {u.status?.simulated && <Badge tone="outline">Simulated feed</Badge>}
          </span>
        }
        description={statusLine}
      >
        {u.status?.state === "available" ? (
          <Button variant="primary" size="sm" onClick={() => u.setDialogOpen(true)}>
            View update
          </Button>
        ) : (
          <Button size="sm" disabled={checking} onClick={() => void u.check(true)}>
            {checking && <Loader2 className="animate-spin" />}
            Check for updates
          </Button>
        )}
      </SettingRow>
    </SettingsGroup>
  )
}
