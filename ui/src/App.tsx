import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import { AppShell } from "@/components/app/AppShell"
import { ProjectProvider } from "@/state/project"
import { SettingsProvider } from "@/state/settings"
import { MenuActionsProvider } from "@/state/menu-actions"
import { DashboardPage } from "@/pages/DashboardPage"
import { ImportPage } from "@/pages/ImportPage"
import { GalleryPage } from "@/pages/GalleryPage"
import { TimelinePage } from "@/pages/TimelinePage"
import { ExportPage } from "@/pages/ExportPage"
import { SettingsPage } from "@/pages/SettingsPage"

export default function App() {
  return (
    <TooltipProvider>
      <ProjectProvider>
        <SettingsProvider>
          <MenuActionsProvider>
            <BrowserRouter>
              <AppShell>
                <Routes>
                  <Route path="/" element={<DashboardPage />} />
                  <Route path="/import" element={<ImportPage />} />
                  <Route path="/gallery" element={<GalleryPage />} />
                  <Route path="/timeline" element={<TimelinePage />} />
                  <Route path="/export" element={<ExportPage />} />
                  <Route path="/settings" element={<SettingsPage />} />
                  <Route path="*" element={<Navigate to="/" replace />} />
                </Routes>
              </AppShell>
            </BrowserRouter>
          </MenuActionsProvider>
          <Toaster position="bottom-right" richColors closeButton />
        </SettingsProvider>
      </ProjectProvider>
    </TooltipProvider>
  )
}
