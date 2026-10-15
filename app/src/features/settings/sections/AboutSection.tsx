import { FileText } from "@/components/icons"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { Button } from "@/components/ui/button"
import { useAppActions } from "@/lib/actions"
import { useAppInfo, useMediaTools } from "@/lib/queries"
import { SettingRow, SettingsGroup } from "../primitives"

const PLATFORM_LABEL = { windows: "Windows", macos: "macOS", linux: "Linux" } as const

/** Third-party components shipped inside the app. */
const LICENSES: { name: string; license: string; url: string }[] = [
  { name: "Tauri", license: "MIT / Apache-2.0", url: "https://tauri.app" },
  { name: "React", license: "MIT", url: "https://react.dev" },
  { name: "Inter", license: "SIL Open Font License 1.1", url: "https://rsms.me/inter" },
  { name: "Instrument Serif", license: "SIL Open Font License 1.1", url: "https://github.com/Instrument/instrument-serif" },
  { name: "Geist Mono", license: "SIL Open Font License 1.1", url: "https://vercel.com/font" },
  { name: "Radix UI", license: "MIT", url: "https://www.radix-ui.com" },
  { name: "Phosphor Icons", license: "MIT", url: "https://phosphoricons.com" },
]

export function AboutSection() {
  const { data: info } = useAppInfo()
  const { data: tools } = useMediaTools()
  const a = useAppActions()
  return (
    <>
      <div className="relative mb-10 flex items-center gap-6 overflow-hidden rounded-3xl border border-white/[0.08] bg-[radial-gradient(120%_140%_at_0%_0%,rgb(224_180_92/0.16),transparent_55%),rgb(255_255_255/0.03)] p-7">
        <AutoSceneMark className="size-20 drop-shadow-[0_12px_32px_rgb(224_180_92/0.35)]" />
        <div className="min-w-0 flex-1">
          <div className="display text-4xl text-fg-1">AutoScene</div>
          <div className="mt-1 text-md text-fg-2">
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

      <SettingsGroup title="Media engine" description="Decoding, indexing and encoding run on FFmpeg.">
        <SettingRow
          label={tools?.ready ? `FFmpeg ${tools.version}` : "FFmpeg"}
          description={
            tools ? (
              tools.ready ? (
                <span data-selectable className="font-mono">{tools.path}</span>
              ) : (
                <span className="text-danger">{tools.error}</span>
              )
            ) : (
              "Checking…"
            )
          }
        >
          <span className="text-sm text-fg-3">LGPL / GPL</span>
        </SettingRow>
      </SettingsGroup>

      <SettingsGroup title="Open-source licenses" description="AutoScene is built on these projects.">
        {LICENSES.map((l) => (
          <SettingRow key={l.name} label={l.name} description={l.url} className="min-h-12 py-2.5">
            <span className="text-sm text-fg-3">{l.license}</span>
          </SettingRow>
        ))}
      </SettingsGroup>
    </>
  )
}
