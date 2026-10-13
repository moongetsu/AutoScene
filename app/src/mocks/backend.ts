/**
 * Browser mock backend. When the UI runs outside Tauri (plain `vite`, Vitest,
 * Playwright), IPC calls are answered here so every screen is reachable
 * without the Rust side. Keep handlers shaped exactly like the real commands —
 * `bindings.ts` types are the contract.
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks"
import type { AppInfo, UpdateStatus } from "@/bindings"

type Handler = (args: Record<string, unknown>) => unknown

const appInfo: AppInfo = {
  version: "0.2.0-dev",
  platform: navigator.userAgent.includes("Mac") ? "macos" : "windows",
  arch: "x86_64",
  dataDir: String.raw`C:\Users\you\AppData\Local\AutoScene`,
  logsDir: String.raw`C:\Users\you\AppData\Local\AutoScene\logs`,
  libraryRoot: String.raw`C:\Users\you\Documents\AutoScene Library`,
}

const handlers: Record<string, Handler> = {
  app_info: () => appInfo,
  reveal_path: () => null,
  open_logs_dir: () => null,
  update_check: (): UpdateStatus => ({
    state: "current",
    currentVersion: appInfo.version,
    version: null,
    notes: null,
    date: null,
    simulated: true,
  }),
  update_install: () => null,
  update_relaunch: () => false,
}

export function installMockBackend() {
  mockWindows("main")
  mockIPC(
    (cmd, payload) => {
      // Built-in plugin calls (window controls, dialogs) are no-ops in the browser.
      if (cmd.startsWith("plugin:")) return null
      const h = handlers[cmd]
      if (!h) {
        console.warn(`[mock] unhandled command ${cmd}`, payload)
        throw { code: "not_found", message: `Mock backend has no handler for ${cmd}` }
      }
      return h((payload ?? {}) as Record<string, unknown>)
    },
    { shouldMockEvents: true },
  )
  console.info("[mock] AutoScene mock backend installed")
}
