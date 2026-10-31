// @ts-check
import fs from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const OUTPUT_FILE = path.resolve(__dirname, "../src/features/settings/sections/contributors.json")

const REPO = "tcmmi/AutoScene"
const API_URL = `https://api.github.com/repos/${REPO}/contributors`

/**
 * Filter rules to ignore AI models, bots, and automated service accounts:
 * - GitHub accounts marked with type !== "User" (e.g. "Bot")
 * - Usernames matching known AI agents / bot patterns:
 *   - "[bot]" suffix (e.g. dependabot[bot], github-actions[bot])
 *   - devin, sweep, codegen, copilot, renovate, cursor, claude, gemini, chatgpt
 */
const BOT_PATTERNS = [
  /\[bot\]$/i,
  /^devin/i,
  /^sweep/i,
  /^codegen/i,
  /^github-actions/i,
  /^dependabot/i,
  /^renovate/i,
]

/**
 * Known roles for human contributors to preserve rich descriptions.
 * Any new contributor detected via API is automatically added with "Contributor" role.
 * @type {Record<string, { name?: string; role: string }>}
 */
const KNOWN_ROLES = {
  tcmmi: {
    name: "tcmmi",
    role: "Creator, media engine, models, and core architecture",
  },
  "0nsku": {
    name: "0nsku",
    role: "Core engineering, anime benchmarks, and analysis",
  },
  ihyledd: {
    name: "ihyledd",
    role: "macOS port, settings redesign, and project layout",
  },
  moongetsu: {
    name: "Moongetsu",
    role: "UI design, custom context menus, and visual polish",
  },
}

function isAiOrBot(user) {
  if (user.type && user.type !== "User") return true
  const login = user.login ?? ""
  return BOT_PATTERNS.some((pattern) => pattern.test(login))
}

async function fetchContributors() {
  console.log(`Fetching contributors from GitHub for ${REPO}...`)
  const headers = {
    "User-Agent": "AutoScene-Contributors-Sync",
    Accept: "application/vnd.github.v3+json",
  }
  if (process.env.GITHUB_TOKEN) {
    headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  }

  const res = await fetch(API_URL, { headers })
  if (!res.ok) {
    throw new Error(`GitHub API error (${res.status}): ${await res.text()}`)
  }

  /** @type {any[]} */
  const apiContributors = await res.json()
  const humanContributors = apiContributors.filter((u) => !isAiOrBot(u))

  const result = humanContributors.map((u) => {
    const handle = u.login
    const known = KNOWN_ROLES[handle.toLowerCase()]
    return {
      name: known?.name ?? handle,
      handle,
      avatar: u.avatar_url,
      role: known?.role ?? "Open-source contributor",
      profileUrl: u.html_url ?? `https://github.com/${handle}`,
    }
  })

  await fs.writeFile(OUTPUT_FILE, `${JSON.stringify(result, null, 2)}\n`, "utf-8")
  console.log(`Wrote ${result.length} contributors to ${OUTPUT_FILE}`)
}

fetchContributors().catch((err) => {
  console.error("Failed to sync contributors:", err)
  process.exit(1)
})
