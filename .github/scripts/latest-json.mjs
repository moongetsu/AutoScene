// Writes the updater manifest (`latest.json`) for a release from the signed
// update bundles in a folder: `node latest-json.mjs <folder> <tag>`.
//
// Each bundle sits next to its `.sig` (what `tauri build` writes with the
// updater key); the manifest names where the release will serve it. Without
// signatures there is nothing an installed app could verify: no manifest.
import { readdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"

const [dir, tag] = process.argv.slice(2)
if (!dir || !tag?.startsWith("v")) {
  console.error("usage: node latest-json.mjs <folder> <vX.Y.Z>")
  process.exit(2)
}
const repo = process.env.GITHUB_REPOSITORY ?? "tcmmi/AutoScene"

/**
 * Which bundle updates which install. The updater asks for
 * `<os>-<arch>-<installer>` first, then `<os>-<arch>`: on Linux each package
 * kind has its own key, so a .deb install is never offered an AppImage.
 */
const KINDS = [
  { platform: "windows-x86_64", test: (f) => f.endsWith("_x64-setup.exe") },
  { platform: "darwin-aarch64", test: (f) => f.endsWith("_aarch64.app.tar.gz") },
  { platform: "linux-x86_64-appimage", test: (f) => f.endsWith(".AppImage") },
  { platform: "linux-x86_64-deb", test: (f) => f.endsWith(".deb") },
  { platform: "linux-x86_64-rpm", test: (f) => f.endsWith(".rpm") },
]

const files = readdirSync(dir)
const platforms = {}
for (const { platform, test } of KINDS) {
  const bundle = files.find((f) => test(f) && files.includes(`${f}.sig`))
  if (!bundle) continue
  platforms[platform] = {
    signature: readFileSync(join(dir, `${bundle}.sig`), "utf8").trim(),
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(bundle)}`,
  }
}

const found = Object.keys(platforms)
if (found.length === 0) {
  console.log("No signed update bundles: no latest.json (installed apps keep their version).")
  process.exit(0)
}
if (found.length < KINDS.length) {
  console.log(`::warning::Update bundles only for ${found.join(", ")}`)
}
const manifest = {
  version: tag.slice(1),
  notes: `What's new: https://github.com/${repo}/releases/tag/${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
}
writeFileSync(join(dir, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`)
console.log(`latest.json for ${found.join(", ")}`)
