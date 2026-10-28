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

The builds aren't signed by a publisher yet, so the first start needs your OK
once:

- **Windows:** SmartScreen may say it protected your PC. Choose **More info →
  Run anyway**. (With Smart App Control on, Windows 11 offers no such choice:
  it blocks unsigned apps it doesn't know.) The installer fetches WebView2 if
  it's missing.
- **macOS:** drag AutoScene from the disk image to Applications and open it.
  macOS says it can't verify the app: close the message. Then open **System
  Settings → Privacy & Security**, scroll down to the note about AutoScene,
  choose **Open Anyway** and confirm. (Since macOS 15, right-click → Open no
  longer does this.) If macOS calls the app damaged (versions up to 1.0.1
  aren't signed) or later refuses a part of it, such as
  `libonnxruntime.dylib`, run
  `xattr -dr com.apple.quarantine /Applications/AutoScene.app` once.
- **Linux:** make the AppImage executable (`chmod +x AutoScene_*.AppImage`).
  WebKitGTK 4.1 is required for the `.deb` and `.rpm` packages.

FFmpeg and ONNX Runtime ship with the app. The AI models (about 310 MB)
download the first time an analysis needs them. Analysis runs on the GPU
through DirectML on Windows and Core ML on macOS; on Linux it runs on the CPU.

### Check a download

Releases after 1.0.1 come with a signed record for every file (a build
attestation): the workflow of this repository that built it, and the tag and
commit it was built from. To check a file, use the
[GitHub CLI](https://cli.github.com), signed in with any GitHub account
(`gh auth login`):

```bash
gh attestation verify AutoScene_<version>_x64-setup.exe -R tcmmi/AutoScene
```

It shows the workflow and the tag. To insist on the release workflow, add
`--signer-workflow tcmmi/AutoScene/.github/workflows/release.yml`.

They also have a `SHA256SUMS` file with the SHA-256 of every other file. Put
it next to your downloads and run
`sha256sum --check --ignore-missing SHA256SUMS` (Linux) or
`shasum -a 256 --check --ignore-missing SHA256SUMS` (macOS). In PowerShell
on Windows, this prints `True` for an intact download:

```powershell
(Get-FileHash .\AutoScene_<version>_x64-setup.exe).Hash -eq (-split (Select-String -Path SHA256SUMS -Pattern x64-setup.exe -SimpleMatch).Line)[0]
```

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
  later analysis: they go back onto the faces it finds again. One whose face
  it doesn't find waits for the next analysis, and the cast says how many
  wait. Anime characters come in a later version.
- **Scenepacks.** For each character you get their shots, filters and your own
  picks, then one export: a single video with a chapter for each shot, or a
  file per shot, named from a template.
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

The full face pass's models (SCRFD and ArcFace, with the people models)
come from InsightFace and may only be used non-commercially; Fast Scan's
(YuNet, SFace) are MIT and Apache-2.0. The shot detector, TransNetV2, is
MIT-licensed. **Settings → About** shows each model's licence. `models/manifest.toml` lists
the source and licence of each model.
