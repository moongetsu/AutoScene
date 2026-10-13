import { FileText } from "lucide-react"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { Button } from "@/components/ui/button"
import { useAppActions } from "@/lib/actions"
import { useAppInfo } from "@/lib/queries"
import { SettingRow, SettingsGroup } from "../primitives"

const PLATFORM_LABEL = { windows: "Windows", macos: "macOS", linux: "Linux" } as const

/** Third-party components shipped inside the app. */
const LICENSES: { name: string; license: string; url: string }[] = [
  { name: "Tauri", license: "MIT / Apache-2.0", url: "https://tauri.app" },
  { name: "React", license: "MIT", url: "https://react.dev" },
  { name: "Geist & Geist Mono", license: "SIL Open Font License 1.1", url: "https://vercel.com/font" },
  { name: "Radix UI", license: "MIT", url: "https://www.radix-ui.com" },
  { name: "Lucide icons", license: "ISC", url: "https://lucide.dev" },
]

export function AboutSection() {
  const { data: info } = useAppInfo()
  const a = useAppActions()
  return (
    <>
      <div className="mb-8 flex items-center gap-4 rounded-lg border border-hairline bg-panel p-5 shadow-1">
        <AutoSceneMark className="size-14" />
        <div className="min-w-0 flex-1">
          <div className="text-lg font-semibold text-fg-1">AutoScene</div>
          <div className="text-sm text-fg-2">
            Version <span className="timecode text-fg-1">{info?.version ?? "–"}</span>
            {info && (
              <span className="text-fg-3">
                {" "}
                · {PLATFORM_LABEL[info.platform]} {info.arch}
              </span>
            )}
          </div>
        </div>
        <Button size="sm" onClick={() => void a.openLogs()}>
          <FileText /> Open logs
        </Button>
      </div>

      <SettingsGroup title="Open-source licenses" description="AutoScene is built on these projects.">
        {LICENSES.map((l) => (
          <SettingRow key={l.name} label={l.name} description={l.url} className="min-h-12 py-2.5">
            <span className="text-xs text-fg-3">{l.license}</span>
          </SettingRow>
        ))}
      </SettingsGroup>
    </>
  )
}
