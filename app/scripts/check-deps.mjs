// Fails when a package that keeps module-level state is installed more than once.
//
// Radix primitives remember which layers are open (and the `<body>`
// pointer-events they borrowed) per copy of the module. With two copies one
// Escape closed a select together with its dialog and left the window
// unclickable. React, the router and query client and cmdk must be single too.
// A pre-release `radix-ui` makes this easy to get wrong: packages that ask for
// `^1.x` of a primitive can't share its RC, so npm installs both. The fix is
// an `overrides` entry per duplicated primitive (printed below).

import { existsSync, readdirSync, readFileSync } from "node:fs"
import { join, relative } from "node:path"

const app = join(import.meta.dirname, "..")
const SINGLE = [/^@radix-ui\//, /^react$/, /^react-dom$/, /^@tanstack\/react-(query|router)$/, /^cmdk$/]

/** Every installed copy as `[name, version, dir]`, nested ones included. */
function* copies(dir) {
  if (!existsSync(dir)) return
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue
    if (entry.name.startsWith("@")) {
      for (const sub of readdirSync(join(dir, entry.name), { withFileTypes: true })) {
        if (sub.isDirectory()) yield* copy(join(dir, entry.name, sub.name), `${entry.name}/${sub.name}`)
      }
    } else {
      yield* copy(join(dir, entry.name), entry.name)
    }
  }
}

function* copy(dir, name) {
  const manifest = join(dir, "package.json")
  if (existsSync(manifest)) yield [name, JSON.parse(readFileSync(manifest, "utf8")).version, dir]
  yield* copies(join(dir, "node_modules"))
}

const byName = new Map()
for (const [name, version, dir] of copies(join(app, "node_modules"))) {
  if (!SINGLE.some((re) => re.test(name))) continue
  byName.set(name, [...(byName.get(name) ?? []), { version, dir }])
}

const doubled = [...byName].filter(([, found]) => found.length > 1)
if (doubled.length === 0) {
  console.log(`check-deps: ${byName.size} packages with shared state, one copy each`)
  process.exit(0)
}

console.error("check-deps: these packages are installed more than once:\n")
for (const [name, found] of doubled) {
  console.error(`  ${name}`)
  for (const { version, dir } of found) console.error(`    ${version.padEnd(28)} ${relative(app, dir)}`)
}
const radix = join(app, "node_modules", "radix-ui", "package.json")
if (existsSync(radix)) {
  const pinned = JSON.parse(readFileSync(radix, "utf8")).dependencies ?? {}
  const fixes = doubled.filter(([name]) => pinned[name]).map(([name]) => `    "${name}": "${pinned[name]}"`)
  if (fixes.length > 0) {
    console.error(`\nPin them to the versions radix-ui uses, in package.json "overrides":\n\n${fixes.join(",\n")}\n`)
  }
}
process.exit(1)
