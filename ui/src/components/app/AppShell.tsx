import { NavLink, useLocation } from "react-router-dom"
import {
  Clapperboard,
  ChevronRight,
  FolderOpen,
  GalleryVerticalEnd,
  ListVideo,
  Settings2,
  Upload,
  Cpu,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarRail,
  SidebarSeparator,
  SidebarTrigger,
} from "@/components/ui/sidebar"
import { AnimatedSidebarNav } from "@/components/app/AnimatedSidebarNav"
import { AnimatedSidebarMenuItem } from "@/components/app/ExpandedSidebarNav"
import { TitleBar } from "@/components/app/TitleBar"
import { MacMenuBridge } from "@/components/app/MacMenuBridge"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { useProjects } from "@/state/project"
import { isMacDesktop } from "@/lib/desktop"
import { cn } from "@/lib/utils"
import { formatDuration } from "@/lib/api"
import { friendlyProjectStatus } from "@/lib/plainCopy"
import { motion } from "motion/react"

const workflow = [
  { to: "/", label: "Dashboard", icon: FolderOpen, end: true },
  { to: "/import", label: "Import", icon: Upload },
  { to: "/gallery", label: "Gallery", icon: GalleryVerticalEnd },
  { to: "/timeline", label: "Timeline", icon: ListVideo },
  { to: "/export", label: "Export", icon: Clapperboard },
]

const routeTitles: Record<string, string> = {
  "/": "Dashboard",
  "/import": "Import",
  "/gallery": "Gallery",
  "/timeline": "Timeline",
  "/export": "Export",
  "/settings": "Settings",
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const { active, activeId, hydrating, projects, gpu, gpuReady } = useProjects()
  const location = useLocation()
  const pageTitle = routeTitles[location.pathname] ?? "AutoScene"
  const shellProject =
    active ??
    (activeId ? projects.find((p) => p.id === activeId) ?? null : null)
  const showActiveBay = Boolean(shellProject) || (hydrating && Boolean(activeId))
  // macOS uses native window chrome: traffic lights instead of the custom
  // titlebar, and a native menu bar instead of in-app controls.
  const mac = isMacDesktop()

  return (
    <div className="flex h-[100dvh] max-h-[100dvh] w-full flex-col overflow-hidden bg-background">
      {mac ? null : <TitleBar />}
      <SidebarProvider defaultOpen className="min-h-0 flex-1 !h-auto !min-h-0">
        <Sidebar collapsible="icon" variant="inset" className="border-r-0">
          {/*
            The sidebar is viewport-fixed at top:0, so with no titlebar the
            traffic lights land over it. This spacer clears them and doubles as
            the primary drag handle.
          */}
          {mac ? (
            <div aria-hidden data-tauri-drag-region className="h-7 shrink-0 select-none" />
          ) : null}
          <SidebarHeader className="border-b border-sidebar-border/60 py-2">
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton size="lg" asChild tooltip="AutoScene">
                  <NavLink to="/">
                    <motion.div
                      whileHover={{ scale: 1.04 }}
                      whileTap={{ scale: 0.97 }}
                      transition={{ type: "spring", stiffness: 420, damping: 24 }}
                      className="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-background/90 shadow-[0_0_20px_oklch(0.78_0.14_65/0.18)] ring-1 ring-primary/20 [&_svg]:!size-full"
                    >
                      <AutoSceneMark className="block size-full" title="AutoScene" />
                    </motion.div>
                    <div className="grid flex-1 text-left leading-tight">
                      <span className="truncate font-heading text-sm tracking-tight">
                        AutoScene
                      </span>
                      <span className="truncate text-[10px] text-muted-foreground">
                        Scene pack studio
                      </span>
                    </div>
                  </NavLink>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarHeader>

          <SidebarContent className="group-data-[collapsible=icon]:overflow-visible">
            <SidebarGroup>
              <SidebarGroupLabel>Workflow</SidebarGroupLabel>
              <SidebarGroupContent>
                <AnimatedSidebarNav items={workflow} />
              </SidebarGroupContent>
            </SidebarGroup>

            {showActiveBay ? (
              <>
                <SidebarSeparator />
                <SidebarGroup>
                  <SidebarGroupLabel>Active project</SidebarGroupLabel>
                  <SidebarGroupContent>
                    <div className="rounded-lg border border-sidebar-border/60 bg-sidebar-accent/30 px-2.5 py-2 group-data-[collapsible=icon]:hidden">
                      {shellProject ? (
                        <>
                          <p className="truncate text-sm font-medium">
                            {shellProject.name}
                          </p>
                          <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
                            {friendlyProjectStatus(shellProject.status)}
                            {shellProject.duration_sec > 0
                              ? ` · ${formatDuration(shellProject.duration_sec)}`
                              : ""}
                          </p>
                          <div className="mt-1.5 flex flex-wrap gap-1">
                            <Badge
                              variant={
                                shellProject.status === "ready"
                                  ? "default"
                                  : "secondary"
                              }
                              className="text-[10px]"
                            >
                              {friendlyProjectStatus(shellProject.status)}
                            </Badge>
                            {shellProject.people?.length ? (
                              <Badge variant="outline" className="text-[10px]">
                                {shellProject.people.length} cast
                              </Badge>
                            ) : null}
                          </div>
                        </>
                      ) : (
                        <>
                          <p className="truncate text-sm font-medium text-muted-foreground">
                            Restoring project…
                          </p>
                          <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
                            Reconnecting to scan
                          </p>
                        </>
                      )}
                    </div>
                  </SidebarGroupContent>
                </SidebarGroup>
              </>
            ) : null}
          </SidebarContent>

          <SidebarFooter className="border-t border-sidebar-border/60">
            <SidebarMenu>
              <AnimatedSidebarMenuItem
                to="/settings"
                label="Settings"
                isActive={location.pathname === "/settings"}
                icon={Settings2}
              />
            </SidebarMenu>
            {gpuReady ? (
              gpu ? (
                <div className="flex items-center gap-2 px-2 py-1 text-[10px] text-muted-foreground group-data-[collapsible=icon]:hidden">
                  <Cpu className="size-3.5 shrink-0 text-primary/80" />
                  <span className="truncate">
                    {gpu.available ? (gpu.name ?? "Graphics ready") : "Graphics unavailable"}
                  </span>
                </div>
              ) : null
            ) : (
              <div className="flex items-center gap-2 px-2 py-1 text-[10px] text-muted-foreground group-data-[collapsible=icon]:hidden">
                <Cpu className="size-3.5 shrink-0 text-muted-foreground/60" />
                <span className="truncate">Checking graphics…</span>
              </div>
            )}
          </SidebarFooter>
          <SidebarRail />
        </Sidebar>

        {mac ? <MacMenuBridge /> : null}

        <SidebarInset className="app-shell app-grain relative z-10 dark min-h-0 overflow-hidden">
          {/*
            data-tauri-drag-region matches per-element on the event target, so
            the trigger and breadcrumb links below stay clickable.
          */}
          <header
            data-tauri-drag-region={mac ? true : undefined}
            className={cn(
              "flex h-11 shrink-0 items-center gap-2.5 border-b border-border/60 bg-background/50 px-3 backdrop-blur-md",
              mac && "select-none",
            )}
          >
            <SidebarTrigger className="-ml-0.5" />
            <SidebarSeparator orientation="vertical" className="mr-0.5 h-4" />
            <Breadcrumb>
              <BreadcrumbList>
                <BreadcrumbItem className="hidden sm:block">
                    <span className="flex items-center gap-1.5 text-muted-foreground">
                      <AutoSceneMark className="size-3.5" />
                      AutoScene
                    </span>
                </BreadcrumbItem>
                <BreadcrumbSeparator className="hidden sm:block">
                  <ChevronRight className="size-3.5" />
                </BreadcrumbSeparator>
                <BreadcrumbItem>
                  <BreadcrumbPage>{pageTitle}</BreadcrumbPage>
                </BreadcrumbItem>
                {shellProject ? (
                  <>
                    <BreadcrumbSeparator>
                      <ChevronRight className="size-3.5" />
                    </BreadcrumbSeparator>
                    <BreadcrumbItem>
                      <span className="max-w-[160px] truncate text-muted-foreground">
                        {shellProject.name}
                      </span>
                    </BreadcrumbItem>
                  </>
                ) : null}
              </BreadcrumbList>
            </Breadcrumb>
          </header>
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{children}</div>
        </SidebarInset>
      </SidebarProvider>
    </div>
  )
}
