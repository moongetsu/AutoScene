/**
 * Boot gate: keep HTML splash visible until API is stably ready.
 *
 * In Tauri, TCP-probe via Rust first so Chromium never logs
 * net::ERR_CONNECTION_REFUSED during the long engine warm-up.
 */

const BOOT_TIMEOUT_MS = 90_000
const POLL_OK_MS = 400
const POLL_DOWN_MS = 1000
/** Avoid mounting on a one-shot TCP blip while the engine is still coming up. */
const READY_STREAK = 3

const STAGES = [
  { after: 0, text: "Starting…" },
  { after: 4_000, text: "Preparing workspace…" },
  { after: 12_000, text: "Connecting…" },
  { after: 25_000, text: "Almost ready…" },
] as const

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
}

function apiBase(): string {
  if (isTauri()) return "http://127.0.0.1:8741"
  return ""
}

function setStatus(text: string) {
  const el = document.getElementById("boot-status")
  if (el) el.textContent = text
}

function setFailed(failed: boolean) {
  const splash = document.getElementById("boot-splash")
  if (!splash) return
  splash.dataset.failed = failed ? "true" : "false"
  splash.setAttribute("aria-busy", failed ? "false" : "true")
}

export function hideBootSplash() {
  const splash = document.getElementById("boot-splash")
  if (!splash) return
  splash.dataset.hide = "true"
  splash.setAttribute("aria-busy", "false")
  window.setTimeout(() => splash.remove(), 500)
}

type HealthBody = {
  ok?: boolean
  ready?: boolean
  token?: string
  stage?: string
  error?: string | null
}

/** Native TCP check — no browser network log when refused. */
async function portListening(): Promise<boolean> {
  if (!isTauri()) return true
  try {
    const { invoke } = await import("@tauri-apps/api/core")
    return await invoke<boolean>("probe_api_port")
  } catch {
    return false
  }
}

async function probeHealth(): Promise<HealthBody | null> {
  const ctrl = new AbortController()
  const kill = window.setTimeout(() => ctrl.abort(), 2500)
  try {
    const res = await fetch(`${apiBase()}/api/health`, {
      cache: "no-store",
      signal: ctrl.signal,
    })
    if (!res.ok) return null
    return (await res.json()) as HealthBody
  } catch {
    return null
  } finally {
    window.clearTimeout(kill)
  }
}

export function startBootWatch(onReady: () => void): () => void {
  const started = Date.now()
  let cancelled = false
  let streak = 0
  let timer: number | undefined
  let stageTimer: number | undefined

  let lastServerStage = ""

  const tickStages = () => {
    const splash = document.getElementById("boot-splash")
    if (splash?.dataset.failed === "true") return
    if (lastServerStage) {
      setStatus(lastServerStage)
      return
    }
    const elapsed = Date.now() - started
    let text: string = STAGES[0].text
    for (const s of STAGES) {
      if (elapsed >= s.after) text = s.text
    }
    setStatus(text)
  }

  tickStages()
  stageTimer = window.setInterval(tickStages, 1000)

  const fail = () => {
    setFailed(true)
    setStatus("Could not start. Check that AutoScene can run, then retry.")
  }

  const finish = async (token?: string) => {
    const { seedApiToken, markApiReady } = await import("@/lib/api")
    if (token) seedApiToken(token)
    markApiReady()
    if (stageTimer) window.clearInterval(stageTimer)
    setStatus("Loading workspace…")
    onReady()
  }

  const loop = async () => {
    if (cancelled) return
    if (Date.now() - started > BOOT_TIMEOUT_MS) {
      fail()
      return
    }

    // Wait for TCP without fetch — kills CONNECTION_REFUSED console spam.
    const open = await portListening()
    if (cancelled) return
    if (!open) {
      streak = 0
      timer = window.setTimeout(() => void loop(), POLL_DOWN_MS)
      return
    }

    const body = await probeHealth()
    if (cancelled) return

    if (typeof body?.stage === "string" && body.stage.trim()) {
      lastServerStage = body.stage.trim()
      setStatus(lastServerStage)
    }

    if (body?.ok && body.ready === false && body.error) {
      setFailed(true)
      setStatus(body.error)
      return
    }

    // ready === false → port up, startup gate not finished yet.
    const healthy = Boolean(body?.ok) && body?.ready === true
    if (healthy) {
      streak += 1
      if (streak >= READY_STREAK) {
        await finish(typeof body?.token === "string" ? body.token : undefined)
        return
      }
    } else {
      streak = 0
    }

    timer = window.setTimeout(
      () => void loop(),
      healthy ? POLL_OK_MS : POLL_DOWN_MS,
    )
  }

  const retryBtn = document.getElementById("boot-retry")
  const onRetry = () => {
    streak = 0
    setFailed(false)
    setStatus("Starting…")
    void loop()
  }
  retryBtn?.addEventListener("click", onRetry)

  void loop()

  return () => {
    cancelled = true
    if (timer) window.clearTimeout(timer)
    if (stageTimer) window.clearInterval(stageTimer)
    retryBtn?.removeEventListener("click", onRetry)
  }
}
