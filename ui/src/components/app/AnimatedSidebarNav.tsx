import { useSidebar } from "@/components/ui/sidebar"
import { CollapsedSidebarRail, type SidebarMotionItem } from "@/components/app/CollapsedSidebarRail"
import { ExpandedSidebarNav } from "@/components/app/ExpandedSidebarNav"

export function AnimatedSidebarNav({ items }: { items: SidebarMotionItem[] }) {
  const { state, isMobile } = useSidebar()
  const collapsed = state === "collapsed" && !isMobile

  if (collapsed) {
    return <CollapsedSidebarRail items={items} />
  }

  return <ExpandedSidebarNav items={items} />
}
