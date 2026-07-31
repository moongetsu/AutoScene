import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"
import {
  api,
  normalizeSettings,
  type AppSettings,
  type ModelVaultStatus,
} from "@/lib/api"

type SettingsCtx = {
  settings: AppSettings | null
  settingsReady: boolean
  settingsError: string | null
  vault: ModelVaultStatus | null
  vaultReady: boolean
  vaultError: string | null
  refreshSettings: () => Promise<AppSettings | null>
  refreshVault: () => Promise<ModelVaultStatus | null>
  setSettingsLocal: (next: AppSettings) => void
  setVaultLocal: (next: ModelVaultStatus) => void
  saveSettings: (
    patch: Parameters<typeof api.updateSettings>[0],
  ) => Promise<AppSettings>
}

const SettingsContext = createContext<SettingsCtx | null>(null)

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<AppSettings | null>(null)
  const [settingsReady, setSettingsReady] = useState(false)
  const [settingsError, setSettingsError] = useState<string | null>(null)
  const [vault, setVault] = useState<ModelVaultStatus | null>(null)
  const [vaultReady, setVaultReady] = useState(false)
  const [vaultError, setVaultError] = useState<string | null>(null)

  const refreshSettings = useCallback(async () => {
    try {
      const raw = await api.settings()
      const next = normalizeSettings(raw)
      setSettings(next)
      setSettingsError(null)
      return next
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setSettingsError(msg)
      return null
    } finally {
      setSettingsReady(true)
    }
  }, [])

  const refreshVault = useCallback(async () => {
    try {
      const st = await api.modelsStatus()
      setVault(st)
      setVaultError(null)
      return st
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      setVaultError(msg)
      return null
    } finally {
      setVaultReady(true)
    }
  }, [])

  const setSettingsLocal = useCallback((next: AppSettings) => {
    setSettings(next)
    setSettingsError(null)
    setSettingsReady(true)
  }, [])

  const setVaultLocal = useCallback((next: ModelVaultStatus) => {
    setVault(next)
    setVaultError(null)
    setVaultReady(true)
  }, [])

  const saveSettings = useCallback(
    async (patch: Parameters<typeof api.updateSettings>[0]) => {
      const next = normalizeSettings(await api.updateSettings(patch))
      setSettings(next)
      setSettingsError(null)
      setSettingsReady(true)
      return next
    },
    [],
  )

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      const { waitForApiReady } = await import("@/lib/api")
      const ready = await waitForApiReady()
      if (cancelled || !ready) return
      await refreshSettings()
      if (cancelled) return
      await refreshVault()
    })()
    return () => {
      cancelled = true
    }
  }, [refreshSettings, refreshVault])

  const value = useMemo(
    () => ({
      settings,
      settingsReady,
      settingsError,
      vault,
      vaultReady,
      vaultError,
      refreshSettings,
      refreshVault,
      setSettingsLocal,
      setVaultLocal,
      saveSettings,
    }),
    [
      settings,
      settingsReady,
      settingsError,
      vault,
      vaultReady,
      vaultError,
      refreshSettings,
      refreshVault,
      setSettingsLocal,
      setVaultLocal,
      saveSettings,
    ],
  )

  return (
    <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>
  )
}

export function useAppSettings() {
  const ctx = useContext(SettingsContext)
  if (!ctx) throw new Error("useAppSettings needs SettingsProvider")
  return ctx
}
