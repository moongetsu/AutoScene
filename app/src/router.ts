import { createHashHistory, createRootRoute, createRoute, createRouter, redirect } from "@tanstack/react-router"
import { AppShell } from "@/components/shell/AppShell"
import { LibraryPage } from "@/features/library/LibraryPage"
import { SettingsPage } from "@/features/settings/SettingsPage"
import { settingsLabel } from "@/features/settings/sections"
import { SETTINGS_SECTIONS, type SettingsSection } from "@/lib/actions"

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
  interface StaticDataRouteOption {
    /** Breadcrumb label shown in the title bar. */
    crumb?: string | ((params: Record<string, string>) => string)
  }
}

const rootRoute = createRootRoute({ component: AppShell })

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: LibraryPage,
})

const settingsIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  beforeLoad: () => {
    throw redirect({ to: "/settings/$section", params: { section: "general" } })
  },
})

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/$section",
  staticData: { crumb: (p) => `Settings · ${settingsLabel(p.section)}` },
  params: {
    parse: (p) => ({
      section: (SETTINGS_SECTIONS as readonly string[]).includes(p.section) ? (p.section as SettingsSection) : "general",
    }),
    stringify: (p) => ({ section: p.section }),
  },
  component: SettingsPage,
})

const routeTree = rootRoute.addChildren([libraryRoute, settingsIndexRoute, settingsRoute])

// Hash history: the app is served from tauri://localhost with no server-side routing.
export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  defaultPreload: "intent",
  scrollRestoration: true,
})
