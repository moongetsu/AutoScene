# AutoScene

Frame-exact character scenepacks from series and movies. Add a season, let
AutoScene cut it into shots and find who is on screen, then export every
scene of the characters you choose as editing-ready clips.

## Download

Get the newest version from
[Releases](https://github.com/tcmmi/AutoScene/releases/latest).

| System | File |
| --- | --- |
| Windows 10 and 11 (x64) | `AutoScene_<version>_x64-setup.exe` |
| macOS 13 or later on Apple Silicon | `AutoScene_<version>_aarch64.dmg` |
| Linux (x64) | `.AppImage`, `.deb` or `.rpm` |

The builds aren't signed by a publisher yet:

- **Windows:** SmartScreen may say it protected your PC. Choose **More info →
  Run anyway**. The installer fetches WebView2 if it's missing.
- **macOS:** the first launch is refused. Right-click the app and choose
  **Open**, or run `xattr -dr com.apple.quarantine /Applications/AutoScene.app`.
- **Linux:** make the AppImage executable (`chmod +x AutoScene_*.AppImage`).
  WebKitGTK 4.1 is required for the `.deb` and `.rpm` packages.

FFmpeg and ONNX Runtime ship with the app. The AI models (about 310 MB)
download the first time an analysis needs them. Analysis runs on the GPU
through DirectML on Windows and Core ML on macOS; on Linux it runs on the CPU.

## What it does

- **A library of series and movies**, live action and anime. Episodes are
  numbered from their file names, and moved or renamed files can be found
  again.
- **Frame-exact everywhere.** Every frame is indexed from the decoder itself,
  and each export is checked against the source frames.
- **Shots.** A shot detector finds every cut. Transition frames belong to no
  shot, so clips never start mid-dissolve.
- **Characters (live action).** Faces are detected, tracked and grouped into
  characters automatically. You name them, and you review the faces the
  analysis wasn't sure about with one key each. Your decisions survive every
  later analysis. Anime characters come in a later version.
- **Scenepacks.** For each character you get their shots, filters and your own
  picks, then one export with file names from a template.
- **A player for editors.** It has J/K/L shuttle, frame stepping, in and out
  points, and a zoomable timeline with shots and character lanes.

## Build from source

You need:

- **Rust:** the nightly toolchain pinned in `rust-toolchain.toml`. `rustup`
  installs it.
- **Node:** version 26.
- **Windows:** Visual Studio with the C++ tools.
- **Linux:** the WebKitGTK development packages and `patchelf`. See `.github/workflows/ci.yml`.

```bash
cargo xtask fetch-sidecars        # pinned FFmpeg + ONNX Runtime into app/src-tauri
cd app
npm ci
npm run tauri:dev                 # the app with hot reload
npm run tauri:build               # installers in target/release/bundle
```

Tests:

```bash
cargo xtask fixtures              # synthetic test media
cargo test --workspace
cd app && npm test
```

## Layout

| Path | What |
| --- | --- |
| `crates/` | The Rust core: media and frame index (`as-media`), projects and database (`as-project`, `as-db`), analysis engine and inference (`as-engine`, `as-infer`, `as-analysis`, `as-identity`), exports (`as-export`) |
| `app/` | The desktop app: Tauri shell (`app/src-tauri`) and React UI (`app/src`) |
| `xtask/` | Dev tasks: sidecars, fixtures, test models |
| `models/manifest.toml` | The AI models, pinned by URL and SHA-256 |
| `tools/model-prep/` | Dev-only Python that prepares models (never shipped) |
| `docs/DESIGN.md` | The design system |

`src/autoscene/` and `ui/` hold AutoScene 0.1, the Python-based version. They
stay until this version also has its dialogue search, smart tags and frame
interpolation.

## Models and licences

The face models come from InsightFace and may only be used non-commercially.
The shot detector, TransNetV2, is MIT-licensed. `models/manifest.toml` lists
the source and licence of each model.
