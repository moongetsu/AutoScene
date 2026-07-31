# AutoScene

Local Windows desktop app: drop an episode, dense GPU face scan, cluster characters, export scene packs.

Quality-first (InsightFace `antelopev2` default). UI is a Tauri + React shell with a Python GPU API backend.

## Requirements

- Windows 10/11
- Python 3.11+
- Node 20+
- Rust (for Tauri desktop shell)
- FFmpeg on PATH
- **NVIDIA GPU required** (CUDA via `onnxruntime-gpu`). CPU inference is disabled.
- Pip CUDA 13 libs: `nvidia-cublas`, `nvidia-cudnn-cu13`, `nvidia-cuda-runtime`, `nvidia-cuda-nvrtc`, `nvidia-nvjitlink` (pulled by `pip install -e .`)

## Setup

```powershell
cd AutoScene
python -m pip install -e .

cd ui
npm install
npm run build
cd ..
```

Check GPU:

```powershell
autoscene gpu
```

Must show `available: true` / `CUDAExecutionProvider`. If not, install/update NVIDIA drivers and the CUDA pip packages above.

Optional DeepFace A/B backend:

```powershell
python -m pip install -e ".[deepface]"
```

## Run (Tauri desktop — recommended)

From `ui/`:

```powershell
npm run tauri:dev
```

Tauri spawns `autoscene serve --api-only` on port 8741 and loads the React UI in a native window.

One-liner from repo root (starts API if needed, then Tauri):

```powershell
autoscene-desktop --tauri
```

Production build:

```powershell
cd ui
npm run tauri:build
```

## Run (browser dev)

API only:

```powershell
autoscene serve --api-only
```

Vite dev server (proxies `/api`):

```powershell
cd ui
npm run dev
```

Open http://127.0.0.1:5173

Legacy pywebview shell (optional extra):

```powershell
python -m pip install -e ".[desktop]"
autoscene serve
```

## App flow

1. **Import** video (InsightFace antelopev2 / buffalo_l, or DeepFace Facenet512 / ArcFace)
2. Wait for dense scan + HDBSCAN clustering
3. **Gallery** pick character, rename
4. **Timeline** review segments with video preview
5. **Export** per-person clip folder under `%USERPROFILE%\.autoscene\projects\<id>\exports\`

## License note

InsightFace **code** is MIT. Pretrained packs (`antelopev2`, `buffalo_l`, DeepFace `Buffalo_L`) are typically **non-commercial research** unless you obtain InsightFace commercial rights or use self-trained weights. AutoScene is for personal/research use with those packs.
