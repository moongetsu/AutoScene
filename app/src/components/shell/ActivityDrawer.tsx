import { Activity } from "lucide-react"
import { EmptyState } from "@/components/ui/empty-state"
import { Sheet, SheetContent } from "@/components/ui/sheet"
import { useUi } from "@/state/ui"

/** Global view of background work (analysis, proxies, exports, downloads). */
export function ActivityDrawer() {
  const open = useUi((s) => s.activityOpen)
  const setOpen = useUi((s) => s.setActivityOpen)
  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetContent title="Activity">
        <EmptyState
          icon={<Activity />}
          title="Nothing running"
          description="Analysis, proxies, model downloads and exports show up here with live progress while they run."
          className="h-full"
        />
      </SheetContent>
    </Sheet>
  )
}
