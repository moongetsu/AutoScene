"""Model / dependency armory: status, download, optional pip bootstrap."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import logging
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable
from urllib.request import urlretrieve

from autoscene.config import settings

log = logging.getLogger(__name__)

ProgressCb = Callable[[str, float], None]

# Creative product IDs → upstream AnimeCV OML directory / zip stem
ANIMECV_ENCODERS: dict[str, dict[str, Any]] = {
    "cel_canon": {
        "id": "cel_canon",
        "label": "Cel Canon",
        "tagline": "Higher quality",
        "blurb": "Higher quality anime pack. Slower.",
        "role": "quality",
        "zip_name": "0206_resnet152.zip",
        "dir_name": "0206_resnet152",
        "url": (
            "https://github.com/kosuke1701/AnimeCV/releases/download/"
            "0111_best_randaug/0206_resnet152.zip"
        ),
        "sha256": "f9b569ffeaf5edb58d7abb94f1ad161612a5128049920ea05906a0f82b4a24a0",
        "size_bytes": 220518402,
        "approx_mb": 210,
        "pair_threshold": 0.645,
    },
    "ink_flash": {
        "id": "ink_flash",
        "label": "Ink Flash",
        "tagline": "Faster",
        "blurb": "Faster anime pack. Slightly less accurate.",
        "role": "speed",
        "zip_name": "0206_resnet18.zip",
        "dir_name": "0206_resnet18",
        "url": (
            "https://github.com/kosuke1701/AnimeCV/releases/download/"
            "0111_best_randaug/0206_resnet18.zip"
        ),
        "sha256": "6f18c18d771feab82615ef270fcb356432f1d274fd659be4a986b031d964547f",
        "size_bytes": 42472986,
        "approx_mb": 42,
        "pair_threshold": 0.601,
    },
}

INSIGHTFACE_PACKS: dict[str, dict[str, Any]] = {
    "buffalo_l": {
        "id": "buffalo_l",
        "label": "Buffalo L",
        "tagline": "Best everyday match.",
        "blurb": "SCRFD-10GF + ArcFace R50. Live-action / 3D.",
        "role": "quality",
        "approx_mb": 326,
    },
    "buffalo_m": {
        "id": "buffalo_m",
        "label": "Buffalo M",
        "tagline": "Balanced speed and accuracy.",
        "blurb": "Lighter detector, same recognition family.",
        "role": "balanced",
        "approx_mb": 313,
    },
    "buffalo_s": {
        "id": "buffalo_s",
        "label": "Buffalo S",
        "tagline": "Faster scans.",
        "blurb": "Small detector + mobile face. Max FPS.",
        "role": "speed",
        "approx_mb": 159,
    },
    "antelopev2": {
        "id": "antelopev2",
        "label": "Antelope V2",
        "tagline": "Harder faces, slower scans.",
        "blurb": "SCRFD-10GF + Glint R100. Slower, research favorite.",
        "role": "heavy",
        "approx_mb": 407,
    },
}

# Pinned commit — do not float on branch tip.
ANIMECV_GIT = (
    "git+https://github.com/kosuke1701/AnimeCV.git"
    "@dbef15cef3b38d3bb037082319ed147891ed1d96"
)
TORCH_CU_INDEX = "https://download.pytorch.org/whl/cu124"


def models_root() -> Path:
    root = settings.data_dir / "models"
    root.mkdir(parents=True, exist_ok=True)
    return root


def animecv_encoders_root() -> Path:
    p = models_root() / "animecv"
    p.mkdir(parents=True, exist_ok=True)
    return p


def encoder_dir(model_id: str) -> Path:
    meta = ANIMECV_ENCODERS[model_id]
    return animecv_encoders_root() / meta["dir_name"]


def resolve_animecv_model_id(raw: str) -> str:
    """Map creative IDs or upstream dir names → vault id."""
    key = (raw or "").strip().lower()
    if key in ANIMECV_ENCODERS:
        return key
    for mid, meta in ANIMECV_ENCODERS.items():
        if key == meta["dir_name"].lower() or key == meta["zip_name"].lower():
            return mid
    # legacy / unknown → quality default
    return "cel_canon"


def encoder_ready(model_id: str) -> bool:
    mid = resolve_animecv_model_id(model_id)
    d = encoder_dir(mid)
    return (
        (d / "embedder.pth").is_file()
        and (d / "trunk.pth").is_file()
        and (d / "conf.json").is_file()
    )


def insightface_pack_dir(name: str) -> Path:
    return Path.home() / ".insightface" / "models" / name


def insightface_ready(name: str) -> bool:
    d = insightface_pack_dir(name)
    return d.is_dir() and any(d.glob("*.onnx"))


def _torch_lib_dir() -> Path | None:
    """Locate torch/lib without importing torch (avoids loading CUDA DLLs early)."""
    try:
        spec = importlib.util.find_spec("torch")
    except Exception:  # noqa: BLE001
        return None
    if not spec:
        return None
    roots: list[Path] = []
    if spec.submodule_search_locations:
        roots.extend(Path(p) for p in spec.submodule_search_locations)
    if spec.origin:
        roots.append(Path(spec.origin).resolve().parent)
    for root in roots:
        lib = root / "lib"
        if lib.is_dir() and any(lib.glob("*.dll")):
            return lib
    return None


def ensure_torch_dll_path() -> str | None:
    """
    Prefer torch's own CUDA/cuDNN DLLs over nvidia-* wheel bins that ORT may
    have prepended (wrong DLL order → WinError 127 on cudnn_*.dll).
    """
    lib = _torch_lib_dir()
    if not lib:
        return None
    s = str(lib.resolve())
    if hasattr(os, "add_dll_directory"):
        try:
            os.add_dll_directory(s)
        except OSError:
            pass
    path = os.environ.get("PATH", "")
    parts = [p for p in path.split(os.pathsep) if p and p.lower() != s.lower()]
    os.environ["PATH"] = s + os.pathsep + os.pathsep.join(parts)
    return s


def _torch_probe_env() -> dict[str, str]:
    env = os.environ.copy()
    lib = _torch_lib_dir()
    if lib:
        s = str(lib.resolve())
        path = env.get("PATH", "")
        parts = [p for p in path.split(os.pathsep) if p and p.lower() != s.lower()]
        env["PATH"] = s + os.pathsep + os.pathsep.join(parts)
    return env


def _torch_fresh_probe() -> tuple[bool, str]:
    """Probe torch/CUDA in a clean interpreter (ignores this process's cached import)."""
    script = (
        "import torch\n"
        "ver = getattr(torch, '__version__', '?')\n"
        "ok = bool(torch.cuda.is_available())\n"
        "name = torch.cuda.get_device_name(0) if ok else ''\n"
        "print(ver)\n"
        "print('1' if ok else '0')\n"
        "print(name)\n"
    )
    try:
        proc = subprocess.run(
            [sys.executable, "-c", script],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
            env=_torch_probe_env(),
        )
    except Exception as exc:  # noqa: BLE001
        return False, f"torch probe failed: {exc}"
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "probe failed").strip()
        return False, f"torch probe failed: {err[-400:]}"
    lines = [ln.strip() for ln in (proc.stdout or "").splitlines() if ln.strip()]
    if len(lines) < 2:
        return False, "torch probe returned empty output"
    ver, flag = lines[0], lines[1]
    name = lines[2] if len(lines) > 2 else ""
    if flag != "1":
        return False, f"torch {ver} (CUDA unavailable)"
    if name:
        return True, f"torch {ver} · {name}"
    return True, f"torch {ver} · CUDA"


def torch_cuda_ready() -> tuple[bool, str]:
    """In-process readiness: what this running API worker can use right now."""
    ensure_torch_dll_path()
    try:
        import torch
    except ImportError:
        return False, "torch not installed"
    except OSError as exc:
        return False, f"torch DLL load failed: {exc}"
    except Exception as exc:  # noqa: BLE001
        return False, f"torch import failed: {exc}"
    ver = getattr(torch, "__version__", "?")
    if not torch.cuda.is_available():
        return False, f"torch {ver} (CUDA unavailable)"
    try:
        name = torch.cuda.get_device_name(0)
    except Exception:  # noqa: BLE001
        name = "cuda:0"
    return True, f"torch {ver} · {name}"


def torch_cuda_site_ready() -> tuple[bool, str]:
    """Site-packages readiness via a fresh Python process."""
    return _torch_fresh_probe()


def animecv_import_ready() -> tuple[bool, str]:
    try:
        from animecv.object_detection import FaceDetector_EfficientDet  # noqa: F401
        from animecv.general import OML_ImageFolder_Pretrained  # noqa: F401
        from animecv.module import ImageBBEncoder  # noqa: F401
    except Exception as exc:  # noqa: BLE001
        return False, f"animecv import failed: {exc}"
    return True, "animecv import ok"


def scenedetect_ready() -> tuple[bool, str]:
    try:
        from scenedetect import open_video  # noqa: F401
        from scenedetect.detectors import AdaptiveDetector  # noqa: F401
    except Exception as exc:  # noqa: BLE001
        return False, f"scenedetect missing: {exc}"
    try:
        import scenedetect as sd

        ver = getattr(sd, "__version__", "?")
    except Exception:  # noqa: BLE001
        ver = "?"
    return True, f"scenedetect {ver}"


def transnet_ready() -> tuple[bool, str]:
    """Package presence only — never import torch here (blocks API boot)."""
    if importlib.util.find_spec("transnetv2_pytorch") is None:
        return False, "transnetv2-pytorch missing"
    return True, "transnetv2-pytorch installed"


def deepface_ready() -> tuple[bool, str]:
    """Package presence only — full DeepFace import pulls TF/weights lazily."""
    if importlib.util.find_spec("deepface") is None:
        return False, "deepface missing"
    # TF 2.16+ / Keras 3: retina-face needs the tf-keras compatibility package.
    if importlib.util.find_spec("tf_keras") is None:
        return False, "tf-keras missing (required by DeepFace RetinaFace)"
    return True, "deepface + tf-keras installed"


def detector_weights_ready() -> bool:
    # AnimeCV downloads under ~/.animecv by default
    root = Path.home() / ".animecv"
    return any(root.rglob("character-face-efficientdet-c*.pth")) if root.exists() else False


@dataclass
class InstallJob:
    id: str
    status: str = "idle"  # idle|running|done|error
    stage: str = ""
    progress: float = 0.0
    error: str | None = None
    log_lines: list[str] = field(default_factory=list)
    started_at: float | None = None
    finished_at: float | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "status": self.status,
            "stage": self.stage,
            "progress": self.progress,
            "error": self.error,
            "log": self.log_lines[-40:],
            "started_at": self.started_at,
            "finished_at": self.finished_at,
        }


_job_lock = threading.Lock()
_job = InstallJob(id="none")
_job_thread: threading.Thread | None = None


def current_job() -> InstallJob:
    return _job


def vault_status() -> dict[str, Any]:
    site_ok, site_detail = torch_cuda_site_ready()
    proc_ok, proc_detail = torch_cuda_ready()
    torch_ok = site_ok or proc_ok
    if proc_ok:
        torch_detail = proc_detail
    elif site_ok and "DLL" in proc_detail:
        torch_detail = (
            f"{site_detail}. This API process failed to load torch DLLs "
            f"({proc_detail}). Restart API after update; if it persists, "
            f"ORT nvidia-* PATH is conflicting with torch/lib."
        )
    elif site_ok:
        torch_detail = (
            f"{site_detail}. Restart the AutoScene API to load CUDA torch "
            f"(this process still has: {proc_detail})"
        )
    else:
        torch_detail = site_detail or proc_detail
    acv_ok, acv_detail = animecv_import_ready()
    tn_ok, tn_detail = transnet_ready()
    df_ok, df_detail = deepface_ready()
    packs: list[dict[str, Any]] = []
    for mid, meta in ANIMECV_ENCODERS.items():
        packs.append(
            {
                **meta,
                "family": "animecv",
                "installed": encoder_ready(mid),
                "path": str(encoder_dir(mid)),
            }
        )
    for mid, meta in INSIGHTFACE_PACKS.items():
        packs.append(
            {
                **meta,
                "family": "insightface",
                "installed": insightface_ready(mid),
                "path": str(insightface_pack_dir(mid)),
            }
        )
    deps = [
        {
            "id": "torch_cuda",
            "label": "PyTorch CUDA",
            "tagline": "AnimeCV + TransNet GPU runtime",
            "installed": torch_ok,
            "detail": torch_detail,
        },
        {
            "id": "animecv_pkg",
            "label": "AnimeCV package",
            "tagline": "EfficientDet + OML loaders",
            "installed": acv_ok,
            "detail": acv_detail,
        },
        {
            "id": "animecv_detector",
            "label": "Cel detector weights",
            "tagline": "EfficientDet character-face C0/C2",
            "installed": detector_weights_ready(),
            "detail": str(Path.home() / ".animecv"),
        },
        {
            "id": "transnet",
            "label": "TransNetV2",
            "tagline": "GPU scene cuts (CUDA only)",
            "installed": tn_ok,
            "detail": tn_detail,
        },
        {
            "id": "deepface",
            "label": "DeepFace",
            "tagline": "Compare mode (Facenet512 / ArcFace)",
            "installed": df_ok,
            "detail": df_detail,
        },
    ]
    installed_n = sum(1 for p in packs if p["installed"]) + sum(
        1 for d in deps if d["installed"]
    )
    total_n = len(packs) + len(deps)
    return {
        "deps": deps,
        "packs": packs,
        "animecv_models": [
            {**m, "installed": encoder_ready(m["id"])}
            for m in ANIMECV_ENCODERS.values()
        ],
        "ready_count": installed_n,
        "total_count": total_n,
        "all_ready": installed_n == total_n,
        "job": current_job().to_dict(),
        "root": str(models_root()),
    }


def _job_log(msg: str) -> None:
    log.info("%s", msg)
    _job.log_lines.append(msg)
    if len(_job.log_lines) > 200:
        del _job.log_lines[:-120]


def _set_stage(stage: str, progress: float) -> None:
    _job.stage = stage
    _job.progress = max(0.0, min(1.0, progress))


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            chunk = f.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def _verify_sha256(path: Path, expected: str, *, size_bytes: int | None = None) -> None:
    if size_bytes is not None and path.stat().st_size != int(size_bytes):
        raise RuntimeError(
            f"Download size mismatch for {path.name}: "
            f"got {path.stat().st_size}, expected {size_bytes}"
        )
    got = _sha256_file(path)
    if got.lower() != expected.lower():
        raise RuntimeError(
            f"SHA256 mismatch for {path.name}: got {got}, expected {expected}"
        )


def _download(url: str, dest: Path, stage: str, p0: float, p1: float) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")

    def _hook(block: int, block_size: int, total: int) -> None:
        if total <= 0:
            return
        done = min(total, block * block_size)
        frac = done / total
        _set_stage(f"{stage} · {done // (1024 * 1024)}MB", p0 + (p1 - p0) * frac)

    _job_log(f"Downloading {url}")
    urlretrieve(url, str(tmp), reporthook=_hook)
    tmp.replace(dest)


def ensure_encoder_weights(model_id: str, on_progress: ProgressCb | None = None) -> Path:
    mid = resolve_animecv_model_id(model_id)
    meta = ANIMECV_ENCODERS[mid]
    out = encoder_dir(mid)
    if encoder_ready(mid):
        return out
    if on_progress:
        on_progress(f"Downloading {meta['label']}", 0.05)
    zpath = animecv_encoders_root() / meta["zip_name"]
    need_fetch = True
    if zpath.exists():
        try:
            _verify_sha256(
                zpath,
                str(meta["sha256"]),
                size_bytes=int(meta.get("size_bytes") or 0) or None,
            )
            need_fetch = False
        except RuntimeError:
            _job_log(f"Bad cache for {zpath.name}; re-downloading")
            zpath.unlink(missing_ok=True)
    if need_fetch:
        _download(meta["url"], zpath, f"Fetch {meta['label']}", 0.05, 0.7)
        _verify_sha256(
            zpath,
            str(meta["sha256"]),
            size_bytes=int(meta.get("size_bytes") or 0) or None,
        )
    if on_progress:
        on_progress(f"Extracting {meta['label']}", 0.75)
    out.mkdir(parents=True, exist_ok=True)
    from autoscene.security import safe_zip_extract

    safe_zip_extract(zpath, animecv_encoders_root())
    # Some zips nest an extra folder; normalize
    if not encoder_ready(mid):
        nested = animecv_encoders_root() / meta["dir_name"]
        if nested.is_dir():
            pass
        else:
            # find conf.json
            for conf in animecv_encoders_root().rglob("conf.json"):
                if meta["dir_name"] in str(conf.parent):
                    break
            else:
                raise RuntimeError(
                    f"AnimeCV encoder extract incomplete for {mid} under {animecv_encoders_root()}"
                )
    if not encoder_ready(mid):
        raise RuntimeError(f"AnimeCV encoder files missing after extract: {out}")
    if on_progress:
        on_progress(f"{meta['label']} ready", 1.0)
    return out


def _pip(args: list[str], stage: str) -> None:
    cmd = [sys.executable, "-m", "pip", "install", "--upgrade", *args]
    _job_log(" ".join(cmd))
    _set_stage(stage, _job.progress)
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.stdout:
        for line in proc.stdout.splitlines()[-12:]:
            _job_log(line)
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "pip failed").strip()
        raise RuntimeError(f"{stage} failed: {err[-800:]}")


def ensure_torch_cuda() -> None:
    # Prefer a fresh process: pip can replace wheels while this API still holds
    # an older torch import (often 2.x+cpu) until restart.
    site_ok, site_detail = torch_cuda_site_ready()
    if site_ok:
        proc_ok, proc_detail = torch_cuda_ready()
        if proc_ok:
            _job_log(proc_detail)
        else:
            _job_log(
                f"{site_detail}. CUDA wheels are installed; restart the AutoScene API "
                f"to unload stale torch ({proc_detail})."
            )
        return

    _job_log(f"Torch CUDA missing ({site_detail}); installing cu124 wheels…")
    _pip(
        [
            "torch",
            "torchvision",
            "--index-url",
            TORCH_CU_INDEX,
        ],
        "Installing PyTorch CUDA",
    )
    site_ok, site_detail = torch_cuda_site_ready()
    if not site_ok:
        raise RuntimeError(
            "PyTorch CUDA still unavailable after install (fresh Python probe failed). "
            "Detail: " + site_detail
        )
    proc_ok, proc_detail = torch_cuda_ready()
    if proc_ok:
        _job_log(proc_detail)
    else:
        _job_log(
            f"Installed OK ({site_detail}). Restart the AutoScene API to load CUDA torch. "
            f"This process still has: {proc_detail}"
        )


def ensure_animecv_package() -> None:
    ok, detail = animecv_import_ready()
    if ok:
        _job_log(detail)
        return
    # Curated deps (avoid broken setup.py timm git pin)
    _pip(
        [
            "timm",
            "bounding-box",
            "pycocotools",
            "pyyaml",
            "webcolors",
            "tensorboardX",
            "tqdm",
            "requests",
        ],
        "Installing AnimeCV Python deps",
    )
    _pip([ANIMECV_GIT, "--no-deps"], "Installing AnimeCV package")
    ok, detail = animecv_import_ready()
    if not ok:
        raise RuntimeError(
            "AnimeCV still not importable after install. Restart API. " + detail
        )


def ensure_detector_weights() -> None:
    if detector_weights_ready():
        return
    from animecv.object_detection import FaceDetector_EfficientDet

    # Triggers download into ~/.animecv
    FaceDetector_EfficientDet(coef=0, use_cuda=False)
    FaceDetector_EfficientDet(coef=2, use_cuda=False)


def ensure_insightface_pack(name: str) -> None:
    """Download pack if missing. Empty dirs must not block (insightface skips them)."""
    if insightface_ready(name):
        return
    import shutil

    from insightface.utils.storage import download as insightface_download

    from autoscene.faces.insightface_be import ensure_insightface_pack as flatten_pack

    root = Path.home() / ".insightface"
    model_dir = insightface_pack_dir(name)
    # insightface.utils.storage.download returns early if dir exists — even when empty.
    if model_dir.is_dir() and not any(model_dir.glob("*.onnx")):
        _job_log(f"Removing empty InsightFace dir {model_dir} so download can run")
        shutil.rmtree(model_dir, ignore_errors=True)

    _job_log(f"Downloading InsightFace pack {name}…")
    insightface_download("models", name, force=True, root=str(root))
    flatten_pack(name, root=root)
    if not insightface_ready(name):
        raise RuntimeError(
            f"InsightFace pack '{name}' has no .onnx in {model_dir} after download"
        )
    _job_log(f"InsightFace {name} ready ({len(list(model_dir.glob('*.onnx')))} onnx)")


def ensure_animecv_runtime(
    model_id: str,
    *,
    auto_pip: bool = True,
    on_progress: ProgressCb | None = None,
) -> Path:
    """Make AnimeCV usable for a scan. Optionally pip-bootstrap."""
    mid = resolve_animecv_model_id(model_id)

    def prog(msg: str, p: float) -> None:
        if on_progress:
            on_progress(msg, p)

    prog("Checking PyTorch CUDA", 0.05)
    # Prefer in-process check — fresh subprocess probe costs several seconds every scan.
    proc_ok, proc_detail = torch_cuda_ready()
    if not proc_ok:
        site_ok, site_detail = torch_cuda_site_ready()
        if not site_ok:
            if not auto_pip:
                raise RuntimeError(
                    "PyTorch CUDA required for AnimeCV. Open Settings → Models → Install all. "
                    + site_detail
                )
            ensure_torch_cuda()
            site_ok, site_detail = torch_cuda_site_ready()
            proc_ok, proc_detail = torch_cuda_ready()
        if site_ok and not proc_ok:
            raise RuntimeError(
                "PyTorch CUDA is installed, but this API process still has the old CPU torch. "
                "Restart the AutoScene API, then retry. "
                f"Site: {site_detail}. Process: {proc_detail}"
            )
        if not proc_ok:
            raise RuntimeError(
                "PyTorch CUDA required for AnimeCV. Open Settings → Models → Install all. "
                + (site_detail or proc_detail)
            )
    prog("Checking AnimeCV package", 0.25)
    ok, detail = animecv_import_ready()
    if not ok:
        if not auto_pip:
            raise RuntimeError(
                "AnimeCV package missing. Open Settings → Model Vault → Install all. "
                + detail
            )
        ensure_animecv_package()
    prog("Ensuring detector weights", 0.45)
    ensure_detector_weights()
    prog("Ensuring encoder weights", 0.6)
    path = ensure_encoder_weights(mid, on_progress=on_progress)
    prog("AnimeCV ready", 1.0)
    return path


def ensure_scenedetect() -> None:
    ok, detail = scenedetect_ready()
    if ok:
        _job_log(detail)
        return
    _job_log(f"PySceneDetect missing ({detail}); installing…")
    _pip(["scenedetect>=0.6.4"], "Installing PySceneDetect")
    ok, detail = scenedetect_ready()
    if not ok:
        raise RuntimeError("PySceneDetect still unavailable after install. " + detail)
    _job_log(detail)


def ensure_transnet() -> None:
    # TransNet inference wants torch; install CUDA torch first when absent.
    site_ok, _ = torch_cuda_site_ready()
    if not site_ok:
        _job_log("TransNet needs PyTorch — ensuring CUDA torch first…")
        ensure_torch_cuda()
    ok, detail = transnet_ready()
    if ok:
        _job_log(detail)
        return
    _job_log(f"TransNetV2 missing ({detail}); installing…")
    _pip(["transnetv2-pytorch>=1.0.5"], "Installing TransNetV2")
    ok, detail = transnet_ready()
    if not ok:
        raise RuntimeError("TransNetV2 still unavailable after install. " + detail)
    _job_log(detail)


def ensure_deepface() -> None:
    ok, detail = deepface_ready()
    if ok:
        _job_log(detail)
        return
    _job_log(f"DeepFace missing ({detail}); installing…")
    _pip(["deepface>=0.0.93", "tf-keras"], "Installing DeepFace")
    ok, detail = deepface_ready()
    if not ok:
        raise RuntimeError("DeepFace still unavailable after install. " + detail)
    _job_log(detail)
    _job_log("Restart AutoScene if Compare mode was already open.")


def _run_install_targets(targets: list[str]) -> None:
    global _job
    items = list(targets)
    if "all" in items:
        items = [
            "torch_cuda",
            "animecv_pkg",
            "animecv_detector",
            "transnet",
            "deepface",
            *ANIMECV_ENCODERS.keys(),
            *INSIGHTFACE_PACKS.keys(),
        ]
    # de-dupe preserve order
    seen: set[str] = set()
    ordered: list[str] = []
    for t in items:
        if t not in seen:
            seen.add(t)
            ordered.append(t)

    n = max(1, len(ordered))
    for i, t in enumerate(ordered):
        base = i / n
        span = 1.0 / n
        _set_stage(f"Installing {t}", base)
        if t == "torch_cuda":
            ensure_torch_cuda()
        elif t == "animecv_pkg":
            ensure_animecv_package()
        elif t == "animecv_detector":
            ensure_animecv_package()
            ensure_detector_weights()
        elif t == "transnet":
            ensure_transnet()
        elif t == "deepface":
            ensure_deepface()
        elif t == "scenedetect":
            # Legacy target — CPU PySceneDetect removed; install TransNet GPU instead.
            _job_log("scenedetect target remapped → TransNetV2 (GPU-only policy)")
            ensure_transnet()
        elif t in ANIMECV_ENCODERS:
            ensure_encoder_weights(
                t,
                on_progress=lambda m, p, b=base, s=span: _set_stage(m, b + s * p),
            )
        elif t in INSIGHTFACE_PACKS:
            _set_stage(f"InsightFace {t}", base + span * 0.2)
            ensure_insightface_pack(t)
        else:
            _job_log(f"Unknown target skipped: {t}")
        _set_stage(f"Done {t}", base + span)
    _set_stage("Vault ready", 1.0)


def start_install(targets: list[str] | None = None) -> InstallJob:
    global _job, _job_thread
    with _job_lock:
        if _job.status == "running":
            return _job
        _job = InstallJob(
            id=f"job-{int(time.time())}",
            status="running",
            stage="Starting",
            progress=0.0,
            started_at=time.time(),
        )
        want = targets or ["all"]

        def worker() -> None:
            global _job
            try:
                _run_install_targets(want)
                _job.status = "done"
                _job.finished_at = time.time()
                _job_log("Install finished")
            except Exception as exc:  # noqa: BLE001
                log.exception("Model vault install failed")
                _job.status = "error"
                _job.error = str(exc)
                _job.finished_at = time.time()
                _job_log(f"ERROR: {exc}")

        _job_thread = threading.Thread(
            target=worker, name="autoscene-model-vault", daemon=True
        )
        _job_thread.start()
        return _job


def patch_torchvision_pretrained() -> None:
    """AnimeCV uses pretrained=True; silence/bridge modern torchvision."""
    try:
        import torchvision.models as models
    except ImportError:
        return

    def _wrap(factory):  # type: ignore[no-untyped-def]
        def wrapped(*args, pretrained=False, **kwargs):  # type: ignore[no-untyped-def]
            if "weights" not in kwargs:
                if pretrained:
                    try:
                        kwargs["weights"] = "DEFAULT"
                    except Exception:  # noqa: BLE001
                        kwargs["weights"] = None
                else:
                    kwargs["weights"] = None
            try:
                return factory(*args, **kwargs)
            except TypeError:
                kwargs.pop("weights", None)
                return factory(*args, pretrained=pretrained, **kwargs)

        return wrapped

    for name in (
        "resnet18",
        "resnet34",
        "resnet50",
        "resnet101",
        "resnet152",
    ):
        if hasattr(models, name):
            setattr(models, name, _wrap(getattr(models, name)))
