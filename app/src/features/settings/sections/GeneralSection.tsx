import { FolderOpen } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { useAppInfo } from "@/lib/queries"
import { commands, errorMessage } from "@/lib/ipc"
import { PathText, SettingRow, SettingsGroup } from "../primitives"

function reveal(path: string) {
  commands.revealPath(path).catch((e) => toast.error("Couldn't open folder", { description: errorMessage(e) }))
}

export function GeneralSection() {
  const { data: info } = useAppInfo()
  return (
    <>
      <SettingsGroup title="Storage" description="Where AutoScene keeps projects, models and logs.">
        <SettingRow label="Library location" description={info && <PathText path={info.libraryRoot} />}>
          <Button size="sm" variant="ghost" disabled={!info} onClick={() => info && reveal(info.libraryRoot)}>
            <FolderOpen /> Reveal
          </Button>
        </SettingRow>
        <SettingRow label="App data" description={info && <PathText path={info.dataDir} />}>
          <Button size="sm" variant="ghost" disabled={!info} onClick={() => info && reveal(info.dataDir)}>
            <FolderOpen /> Reveal
          </Button>
        </SettingRow>
      </SettingsGroup>
    </>
  )
}
