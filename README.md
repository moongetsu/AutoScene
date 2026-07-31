# AutoScene

Local Windows desktop app: drop an episode, dense GPU face scan, cluster characters, export scene packs.

Quality-first (InsightFace `antelopev2` default). UI is a Tauri + React shell with a Python GPU API backend.

## Requirements

Common to all platforms:

- Python 3.11+
- Node 20+
- Rust (for Tauri desktop shell)
- FFmpeg on PATH

### Windows / Linux

- **NVIDIA GPU required** (CUDA via `onnxruntime-gpu`). CPU inference is disabled.
- Pip CUDA 13 libs: `nvidia-cublas`, `nvidia-cudnn-cu13`, `nvidia-cuda-runtime`, `nvidia-cuda-nvrtc`, `nvidia-nvjitlink` (pulled by `pip install -e .`)

### macOS (Apple Silicon)

- **Apple Silicon (M1 or newer).** Inference runs on the CoreML execution
  provider; there is no `onnxruntime-gpu` build for macOS, so the CUDA wheels
  above are skipped automatically via dependency markers.
- Video encode/decode uses **VideoToolbox** (`h264_videotoolbox` /
  `hevc_videotoolbox`) instead of NVENC. Homebrew's `ffmpeg` includes these.
- Scene-cut detection falls back to **PySceneDetect** on CPU, since the
  TransNetV2 `transnet` extra needs torch. Install the extra to use TransNetV2
  on MPS instead.

## Setup

```bash
cd AutoScene
python3 -m venv .venv && source .venv/bin/activate   # Windows: .venv\Scripts\activate
python -m pip install -e .

cd ui
npm install
npm run build
cd ..
```

Check GPU:

```bash
autoscene gpu
```

Windows/Linux must show `available: true` / `CUDAExecutionProvider`. If not,
install/update NVIDIA drivers and the CUDA pip packages above.

macOS should show `available: true` / `CoreMLExecutionProvider`:

```json
{
  "available": true,
  "name": "Apple M2",
  "provider": "CoreMLExecutionProvider",
  "detail": "CoreMLExecutionProvider session ok"
}
```

A `CPUExecutionProvider` result there means CoreML could not build a session —
it still runs, roughly 6x slower on the scan phase.

Optional DeepFace A/B backend:

```bash
python -m pip install -e ".[deepface]"
```

## Run (Tauri desktop — recommended)

From `ui/`:

```bash
npm run tauri:dev
```

Tauri spawns `autoscene serve --api-only` on port 8741 and loads the React UI in a native window.
It resolves `autoscene` from PATH, so run it from an activated virtualenv.

One-liner from repo root (starts API if needed, then Tauri):

```bash
autoscene-desktop --tauri
```

Production build:

```bash
cd ui
npm run tauri:build
```

## Run (browser dev)

API only:

```bash
autoscene serve --api-only
```

Vite dev server (proxies `/api`):

```bash
cd ui
npm run dev
```

Open http://127.0.0.1:5173

Legacy pywebview shell (optional extra):

```bash
python -m pip install -e ".[desktop]"
autoscene serve
```

## App flow

1. **Import** video (InsightFace antelopev2 / buffalo_l, or DeepFace Facenet512 / ArcFace)
2. Wait for dense scan + HDBSCAN clustering
3. **Gallery** pick character, rename
4. **Timeline** review segments with video preview
5. **Export** per-person clip folder under `~/.autoscene/projects/<id>/exports/`
   (`%USERPROFILE%\.autoscene\...` on Windows)

## License note

InsightFace **code** is MIT. Pretrained packs (`antelopev2`, `buffalo_l`, DeepFace `Buffalo_L`) are typically **non-commercial research** unless you obtain InsightFace commercial rights or use self-trained weights. AutoScene is for personal/research use with those packs.
