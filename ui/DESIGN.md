# Design System: AutoScene

## 1. Visual Theme & Atmosphere
Cockpit-dense post-house bay for character scene packs. Density 7, Variance 6, Motion 6. Charcoal shell, one warm signal accent, monospace timecodes, asymmetric rails vs stages. Clinical precision — film negative + edit suite, not SaaS marketing or neon dashboard cosplay.

**Surfaces:** Boot splash (instant pre-React), Dashboard (workspace landing), Import (ingest bay), Gallery (cast dossier), Timeline (review), Export (reel + cast rail), Settings (device + packs + knobs), App shell (sidebar + title bar).

## 2. Color Palette & Roles
- **Bay Charcoal** (#141210) — Primary app canvas
- **Rail Surface** (#1C1A17) — Side rails, control strips, sidebar depth
- **Clip Plate** (#23201C) — Tiles, elevated plates (only when selected)
- **Bone Ink** (#F2EDE6) — Primary text
- **Dust Steel** (#9A948A) — Secondary text, metadata, helpers
- **Hairline Rule** (rgba(242,237,230,0.10)) — 1px structural dividers
- **Signal Amber** (#C4A35A) — Single accent: CTAs, active rows, focus rings, progress (sat &lt; 80%)
- **Cut Red** (#B54A3A) — Exclude / destructive / off-state only
- **Banned:** pure `#000000`, purple/neon glows, second accent colors, warm/cool gray mixing

## 3. Typography Rules
- **Display / Bay titles:** IBM Plex Sans Variable — track-tight, weight-driven hierarchy
- **Body:** IBM Plex Sans Variable — relaxed leading, Dust Steel for helpers, max ~65ch where prose runs
- **Mono / Timecode:** ui-monospace / JetBrains Mono — durations, in/out, indices, progress counts
- **Copy voice:** Simple user language only. Never name engines, GPUs stacks, codecs, or detection internals in UI chrome
- **Banned:** Inter, generic serifs in this software UI, emoji, AI marketing clichés

## 4. Component Stylings
* **Primary CTA:** Signal Amber fill, Bone Ink text, tactile translateY(1px) on active, no outer glow
* **Ghost / Exclude:** Hairline border, Dust Steel label; excluded tiles desaturate + Cut Red “Off” chip
* **Cards:** Use only when elevation aids hierarchy; otherwise hairline + negative space. No 3-equal marketing card rows
* **Scene tiles:** Dense 16:9 filmstrip grid, index badge top-left, hover end-frame scrub — not equal card rows
* **Cast rail:** Vertical list, active row Signal Amber left hairline, face crop ~40px square
* **Inputs:** Label above, helper optional, error below. Focus ring Signal Amber. No floating labels
* **Loaders:** Skeleton plates matching layout — no circular spinners on reels/grids
* **Empty states:** Composed “what to do next” with one primary CTA (e.g. Import, Gallery)
* **Step rail (Import):** Scenes → Check → Prepare → Find faces → Group → Portraits → Done. Amber fill on current/done

## 5. Layout Principles
- Desktop Import/Export: asymmetric split — main stage `minmax(0,1fr)` + rail `280–320px`
- Gallery/Timeline: dossier + reel; avoid centered marketing heroes inside app chrome
- Mobile `&lt;768px`: single column; rails collapse to horizontal chips; min touch 44px
- Contain content (≈1400–1520px max inside shell). `min-h-[100dvh]` for full-height sections — never `h-screen`
- No overlapping badges on preview media beyond index / off / timecode footer
- CSS Grid preferred; no `calc()` percentage flex hacks

## 6. Motion & Interaction
- Spring default: stiffness 100, damping 20
- Staggered cascade for tile grids (cap ~12 delay steps); transform + opacity only
- Progress bars: Signal Amber fill; stage label soft opacity pulse — no bouncing chevrons
- Hover preview: muted loop on face-trim window; stills use mid face-trim point
- Perpetual micro-loops only on truly active work (scan/export progress), not idle chrome

## 7. Copy & Language (product rule)
User-facing text stays plain and short.
- **Say:** Finding scenes, Graphics ready, Live action, Anime, Characters appear here, Quality encode
- **Never say:** CUDA, TransNet, InsightFace, AnimeCV, DeepFace, NVENC, ffmpeg, onnx, embeddings, cluster math, detector names, frame-budget/probe jargon, stack traces
- Progress may show counts (`12 scenes`, `40%`) without explaining how detection works
- Settings advanced knobs use plain labels (Soft fades, Grouping strength) — hide wire model ids behind friendly pack names
- Errors: short human message; technical detail stays in logs only

## 8. Surface notes
### Boot splash
Full-bleed Bay Charcoal (#141210) with soft Signal Amber radial wash. Brand mark + **AutoScene** wordmark (Bone Ink), one Dust Steel status line, thin Signal Amber indeterminate rail (transform only). No spinner circle. No jargon. Instant HTML paint before React. Fail after 90s → plain Retry CTA. Fade out on health ready (opacity/visibility only).

### Dashboard
One composition: brand-adjacent workspace cue, one headline, one short line, one CTA, recent projects list. No tech status essay — graphics chip optional as Dust Steel meta.

### Import / Ingest bay
Footage plate left, meta rail right. While finding scenes: live mono counter (scenes found), progress %, plain stage line. Never static “Detecting…” with zero numbers. Never fake counts.

### Gallery
Cast tiles + dossier. Empty: characters appear as faces are found. No backend stage dumps.

### Export bay
Reel stage + cast targets rail. Primary CTA = Export merged. End-frame review first-class.

### Settings
Sectioned like a preferences pane, not one long wall. Left rail (General / Faces / Scan quality / Models / System) shows one section at a time; collapses to horizontal chips under `lg`. Every knob is a row: plain label + one-line "what this does" hint on the left, control on the right, amber dot + per-row Reset when edited. Fine-tuning stays behind the header **Advanced** switch — General and Faces must be usable with it off. Content preset (Live-action / Stylized / Anime) is the one choice that drives the rest. Sticky unsaved-changes bar with Discard + Save. License note without naming research packs.

## 9. Anti-Patterns (Banned)
- No emojis, no Inter, no pure black, no neon/outer glows
- No purple accent, no oversaturated CTAs
- No “Scroll to explore” / bouncing arrows
- No 3-column equal marketing cards
- No AI copy (“Elevate”, “Seamless”, “Unleash”, “Next-Gen”)
- No generic placeholder names (John Doe, Acme, Nexus)
- No fake round metrics (99.99%)
- No backend/engine names in UI chrome
- No centered empty marketing hero inside app bays
- No broken Unsplash links — use local assets or picsum/SVG if needed
