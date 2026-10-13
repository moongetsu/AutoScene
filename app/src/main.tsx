import "@fontsource-variable/geist"
import "@fontsource-variable/geist-mono"
import "./styles/index.css"

import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "@tanstack/react-router"
import { Toaster } from "@/components/ui/toaster"
import { TooltipProvider } from "@/components/ui/tooltip"
import { isTauri } from "@/lib/platform"
import { queryClient } from "@/lib/queries"

async function boot() {
  // Outside Tauri (plain browser, tests) answer IPC from the mock backend.
  if (!isTauri) {
    const { installMockBackend } = await import("@/mocks/backend")
    installMockBackend()
  }
  const { router } = await import("./router")

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider delayDuration={400} skipDelayDuration={150}>
          <RouterProvider router={router} />
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    </StrictMode>,
  )

  // Fade the pre-JS splash once React has painted.
  requestAnimationFrame(() => {
    const boot = document.getElementById("boot")
    boot?.setAttribute("data-hide", "")
    window.setTimeout(() => boot?.remove(), 400)
  })
}

void boot()
