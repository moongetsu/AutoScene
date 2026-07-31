import { useEffect, useMemo, useState, type ReactNode } from "react"
import { motion } from "motion/react"
import {
  Clapperboard,
  Download,
  Film,
  Loader2,
  Radio,
  Sparkles,
  Zap,
} from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { api, type ModelVaultStatus } from "@/lib/api"
import { useAppSettings } from "@/state/settings"
import { friendlyError, sanitizeStage } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type PrimeMode = "all" | "anime" | null

export function ModelVault() {
  const {
    vault,
    vaultError,
    vaultReady,
    refreshVault,
    setVaultLocal,
  } = useAppSettings()
  const [busy, setBusy] = useState(false)
  const [bayOpen, setBayOpen] = useState(false)
  const [primeMode, setPrimeMode] = useState<PrimeMode>(null)

  useEffect(() => {
    // Soft refresh; boot cache already paints if ready
    void refreshVault().catch(() => undefined)
  }, [refreshVault])

  const running = vault?.job.status === "running" || busy

  useEffect(() => {
    if (!running) return
    setBayOpen(true)
    const t = window.setInterval(() => {
      void api
        .modelsInstallStatus()
        .then(setVaultLocal)
        .catch(() => undefined)
    }, 1100)
    return () => window.clearInterval(t)
  }, [running, setVaultLocal])

  const install = async (targets: string[], mode: PrimeMode) => {
    setBusy(true)
    setPrimeMode(mode)
    setBayOpen(true)
    try {
      const st = await api.modelsInstall(targets)
      setVaultLocal(st)
    } catch (e) {
      toast.error(friendlyError(e))
      setBayOpen(false)
    } finally {
      setBusy(false)
    }
  }

  const pct = Math.round((vault?.job.progress ?? 0) * 100)
  const armed = vault?.ready_count ?? 0
  const total = vault?.total_count ?? 10
  const charge = total > 0 ? armed / total : 0

  const cells = useMemo(() => {
    if (!vault) return []
    return [
      ...vault.deps.map((d) => ({
        id: d.id,
        label: d.label,
        tagline: d.tagline,
        ready: d.installed,
        family: "runtime" as const,
        meta: d.detail,
      })),
      ...vault.animecv_models.map((m) => ({
        id: m.id,
        label: m.label,
        tagline: m.tagline,
        ready: !!m.installed,
        family: "cel" as const,
        meta: m.blurb,
      })),
      ...vault.packs
        .filter((p) => p.family === "insightface")
        .map((p) => ({
          id: p.id,
          label: p.label,
          tagline: p.tagline,
          ready: p.installed,
          family: "photo" as const,
          meta: `${p.approx_mb ?? "?"}MB`,
        })),
    ]
  }, [vault])

  return (
    <>
      <section className="panel-surface overflow-hidden">
        <header className="border-b border-border/40 px-5 py-4">
          <h2 className="font-heading text-base tracking-tight">Models</h2>
          <p className="mt-1 max-w-[64ch] text-xs leading-relaxed text-muted-foreground">
            Face packs and scene tools AutoScene downloads to your machine. Install everything
            once and you can forget this page.
          </p>
        </header>

        <div className="grid gap-0 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.4fr)]">
          <div className="flex flex-col justify-between border-b border-border/40 p-5 lg:border-b-0 lg:border-r">
            <div className="space-y-4">
              {!vault ? (
                <LoadingBlock
                  label={
                    vaultError || vaultReady
                      ? "Could not load models"
                      : "Loading install status…"
                  }
                  rows={2}
                />
              ) : (
                <>
                  <div className="flex items-end justify-between gap-3">
                    <div>
                      <p className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                        Installed
                      </p>
                      <p className="timecode mt-1 text-3xl tabular-nums text-primary">
                        {armed}
                        <span className="text-lg text-muted-foreground">/{total}</span>
                      </p>
                    </div>
                    <Badge
                      variant="secondary"
                      className={cn(
                        "timecode",
                        vault.all_ready && "border-primary/35 text-primary",
                      )}
                    >
                      {vault.all_ready ? "Ready" : running ? "Installing" : "Incomplete"}
                    </Badge>
                  </div>
                  <Progress value={Math.round(charge * 100)} className="h-1.5" />
                  {running ? (
                    <button
                      type="button"
                      onClick={() => setBayOpen(true)}
                      className="flex w-full items-center gap-2 rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 text-left text-xs text-primary transition hover:bg-primary/10"
                    >
                      <Radio className="size-3.5 animate-pulse" />
                      <span className="truncate">{sanitizeStage(vault.job.stage) || "Installing…"}</span>
                      <span className="timecode ml-auto tabular-nums">{pct}%</span>
                    </button>
                  ) : null}
                </>
              )}

              {vaultError && !vault ? (
                <Button
                  type="button"
                  variant="outline"
                  className="min-h-10 w-full"
                  onClick={() => {
                    void refreshVault().then((st) => {
                      if (!st) toast.error(friendlyError(vaultError, "Could not load models"))
                    })
                  }}
                >
                  Retry
                </Button>
              ) : null}

              <div className="flex flex-col gap-2 sm:flex-row">
                <Button
                  type="button"
                  disabled={running}
                  className="min-h-11 flex-1 gap-2"
                  onClick={() => void install(["all"], "all")}
                >
                  {running && primeMode === "all" ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Zap className="size-4" />
                  )}
                  Install all
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={running}
                  className="min-h-11 flex-1 gap-2"
                  onClick={() =>
                    void install(
                      [
                        "torch_cuda",
                        "animecv_pkg",
                        "animecv_detector",
                        "cel_canon",
                        "ink_flash",
                      ],
                      "anime",
                    )
                  }
                >
                  <Sparkles className="size-4 text-primary" />
                  Anime only
                </Button>
              </div>
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                Install all covers live-action and anime. Downloads run in the background and
                can take a while.
              </p>
            </div>
          </div>

          {/* Rack */}
          <div className="p-5 lg:p-6">
            {!vault ? (
              <LoadingBlock
                label={
                  vaultError
                    ? "Model list unavailable — is AutoScene running?"
                    : "Loading model list…"
                }
                rows={7}
                dense
              />
            ) : (
              <div className="space-y-5">
                <RackBlock
                  title="Core tools"
                  icon={<Radio className="size-3.5" />}
                  items={cells.filter((c) => c.family === "runtime")}
                  running={running}
                  onPull={(id) => void install([id], null)}
                />
                <RackBlock
                  title="Anime models"
                  icon={<Clapperboard className="size-3.5" />}
                  items={cells.filter((c) => c.family === "cel")}
                  running={running}
                  onPull={(id) => void install([id], null)}
                  accent
                />
                <RackBlock
                  title="Live-action models"
                  icon={<Film className="size-3.5" />}
                  items={cells.filter((c) => c.family === "photo")}
                  running={running}
                  onPull={(id) => void install([id], null)}
                />
              </div>
            )}
          </div>
        </div>
      </section>

      <PrimeBaySheet
        open={bayOpen}
        onOpenChange={setBayOpen}
        vault={vault}
        pct={pct}
        cells={cells}
        running={running}
        primeMode={primeMode}
      />
    </>
  )
}

function RackBlock({
  title,
  icon,
  items,
  running,
  onPull,
  accent,
}: {
  title: string
  icon: ReactNode
  items: {
    id: string
    label: string
    tagline: string
    ready: boolean
    meta: string
  }[]
  running: boolean
  onPull: (id: string) => void
  accent?: boolean
}) {
  return (
    <div>
      <p className="mb-2 flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
        {icon}
        {title}
      </p>
      <div className="divide-y divide-border/35 rounded-xl border border-border/40 bg-background/25">
        {items.map((item, i) => (
          <motion.div
            key={item.id}
            initial={{ opacity: 0, x: 8 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ delay: i * 0.04, type: "spring", stiffness: 320, damping: 28 }}
            className={cn(
              "flex items-center gap-3 px-3 py-2.5",
              item.ready && "bg-primary/[0.04]",
              accent && !item.ready && "bg-transparent",
            )}
          >
            <span
              className={cn(
                "size-2 shrink-0 rounded-full",
                item.ready ? "bg-primary shadow-[0_0_8px_oklch(0.78_0.14_65/0.45)]" : "bg-muted-foreground/35",
              )}
            />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">{item.label}</p>
              <p className="truncate text-[11px] text-muted-foreground">{item.tagline}</p>
            </div>
            <Badge
              variant="secondary"
              className={cn(
                "timecode shrink-0 text-[10px]",
                item.ready && "border-primary/30 text-primary",
              )}
            >
              {item.ready ? "Ready" : "Missing"}
            </Badge>
            {!item.ready ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={running}
                className="h-8 shrink-0 px-2 text-xs"
                onClick={() => onPull(item.id)}
              >
                <Download className="size-3.5" />
                Install
              </Button>
            ) : null}
          </motion.div>
        ))}
      </div>
    </div>
  )
}

function PrimeBaySheet({
  open,
  onOpenChange,
  vault,
  pct,
  cells,
  running,
  primeMode,
}: {
  open: boolean
  onOpenChange: (o: boolean) => void
  vault: ModelVaultStatus | null
  pct: number
  cells: { id: string; label: string; ready: boolean; family: string }[]
  running: boolean
  primeMode: PrimeMode
}) {
  const stage = sanitizeStage(vault?.job.stage) || (running ? "Installing…" : "Idle")
  const err = vault?.job.error ? friendlyError(vault.job.error, "Install failed. Try again.") : null
  const done = vault?.job.status === "done"
  const failed = vault?.job.status === "error"

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full gap-0 border-l border-border/50 bg-[oklch(0.09_0.015_280)] p-0 sm:max-w-md"
      >
        <SheetHeader className="border-b border-border/40 px-5 py-5 text-left">
          <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
            Install
          </p>
          <SheetTitle className="font-heading text-xl tracking-tight">
            {primeMode === "anime"
              ? "Anime models"
              : primeMode === "all"
                ? "Install all"
                : "Install model"}
          </SheetTitle>
          <SheetDescription className="text-sm text-muted-foreground">
            Large downloads may sit at 0% for a while. That is normal.
          </SheetDescription>
        </SheetHeader>

        <div className="flex flex-1 flex-col gap-5 overflow-hidden p-5">
          <div className="space-y-3">
            <div className="flex items-end justify-between gap-3">
              <div className="min-w-0">
                <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
                  Stage
                </p>
                <p className="mt-1 truncate text-sm font-medium">{stage}</p>
              </div>
              <p className="timecode text-2xl tabular-nums text-primary">{pct}%</p>
            </div>
            <div className="relative">
              <Progress value={pct} className="h-2" />
              {running && pct < 3 ? (
                <motion.div
                  aria-hidden
                  className="pointer-events-none absolute inset-0 overflow-hidden rounded-full"
                >
                  <motion.div
                    className="h-full w-1/3 bg-primary/25"
                    animate={{ x: ["-100%", "300%"] }}
                    transition={{ duration: 1.6, repeat: Infinity, ease: "linear" }}
                  />
                </motion.div>
              ) : null}
            </div>
            {err ? <p className="text-xs text-destructive">{err}</p> : null}
            {done ? (
              <p className="text-xs text-primary">
                Done. Restart AutoScene if graphics or anime tools were just installed.
              </p>
            ) : null}
            {failed ? (
              <p className="text-xs text-muted-foreground">
                Fix the error above, then try Install again from Models.
              </p>
            ) : null}
          </div>

          <div>
            <p className="mb-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
              Status
            </p>
            <div className="grid grid-cols-3 gap-1.5">
              {cells.map((c) => (
                <div
                  key={c.id}
                  className={cn(
                    "rounded-md border px-2 py-2",
                    c.ready
                      ? "border-primary/35 bg-primary/10"
                      : "border-border/35 bg-background/30",
                  )}
                >
                  <p className="truncate text-[10px] font-medium">{c.label}</p>
                  <p className="timecode mt-0.5 text-[9px] text-muted-foreground">
                    {c.ready ? "OK" : "…"}
                  </p>
                </div>
              ))}
            </div>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  )
}
