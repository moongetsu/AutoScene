import type * as React from "react"
import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  redirect,
} from "@tanstack/react-router"
import { AppShell } from "@/components/shell/AppShell"
import { CastPage } from "@/features/cast/CastPage"
import { CharacterPage } from "@/features/cast/CharacterPage"
import { ReviewPage } from "@/features/cast/ReviewPage"
import { LibraryPage } from "@/features/library/LibraryPage"
import { PlayerPage } from "@/features/player/PlayerPage"
import { EpisodesPage } from "@/features/project/EpisodesPage"
import { ExportsPage } from "@/features/project/ExportsPage"
import { OverviewPage } from "@/features/project/OverviewPage"
import { CharacterCrumb, MediaCrumb, PackCrumb, ProjectCrumb } from "@/features/project/crumbs"
import { SettingsPage } from "@/features/settings/SettingsPage"
import { settingsLabel } from "@/features/settings/sections"
import { NotFound } from "@/features/not-found"
import { PackPage } from "@/features/pack/PackPage"
import { RouteError } from "@/features/route-error"
import { SETTINGS_SECTIONS, type SettingsSection } from "@/lib/actions"

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router
  }
  interface StaticDataRouteOption {
    /** Name of the view in the window title (see WindowTitle). */
    crumb?: React.ReactNode | ((params: Record<string, string>) => React.ReactNode)
  }
}

const rootRoute = createRootRoute({ component: AppShell })

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: LibraryPage,
})

const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/p/$pid",
  staticData: { crumb: (p) => <ProjectCrumb pid={p.pid} /> },
  component: Outlet,
})

const projectIndexRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/",
  component: OverviewPage,
})

const episodesRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/episodes",
  component: EpisodesPage,
})

const playerRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/timeline/$mid",
  staticData: { crumb: (p) => <MediaCrumb pid={p.pid} mid={p.mid} /> },
  // `f`: frame to open at (from a face in the cast).
  validateSearch: (s: Record<string, unknown>): { f?: number } => {
    const f = Number(s.f)
    return Number.isInteger(f) && f >= 0 ? { f } : {}
  },
  component: PlayerPage,
})

const castRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/cast",
  staticData: { crumb: "Cast" },
  component: CastPage,
})

const reviewRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/cast/review",
  staticData: { crumb: "Cast · Review" },
  // `c`: only the faces suggested for this character.
  validateSearch: (s: Record<string, unknown>): { c?: string } => (typeof s.c === "string" ? { c: s.c } : {}),
  component: ReviewPage,
})

const characterRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/cast/$cid",
  staticData: { crumb: (p) => <CharacterCrumb pid={p.pid} cid={p.cid} /> },
  component: CharacterPage,
})

const packRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/cast/$cid/pack",
  staticData: { crumb: (p) => <PackCrumb pid={p.pid} cid={p.cid} /> },
  component: PackPage,
})

const exportsRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/exports",
  staticData: { crumb: "Exports" },
  component: ExportsPage,
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

const routeTree = rootRoute.addChildren([
  libraryRoute,
  projectRoute.addChildren([
    projectIndexRoute,
    episodesRoute,
    playerRoute,
    castRoute,
    reviewRoute,
    characterRoute,
    packRoute,
    exportsRoute,
  ]),
  settingsIndexRoute,
  settingsRoute,
])

// Hash history: the app is served from tauri://localhost with no server-side routing.
export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  defaultPreload: "intent",
  scrollRestoration: true,
  defaultNotFoundComponent: NotFound,
  defaultErrorComponent: RouteError,
})
