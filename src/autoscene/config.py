from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

from pydantic_settings import BaseSettings, SettingsConfigDict

log = logging.getLogger(__name__)

# Keys users can change at runtime (persisted under data_dir/settings.json)
RUNTIME_SETTING_KEYS = (
    "sample_fps",
    "det_size",
    "detect_max_edge",
    "min_face_size",
    "cluster_min_size",
    "cluster_merge_sim",
    "content_preset",
    "prefetch_depth",
    "face_select_mode",
    "face_relative_pct",
    "face_top_n",
    "default_backend",
    "default_model",
    "scene_cut_detector",
    "scene_cut_adaptive_threshold",
    "scene_cut_transnet_threshold",
    "decode_backend",
)

FACE_SELECT_MODES = frozenset({"all", "largest", "relative", "top_n"})
SCENE_CUT_MODES = frozenset({"adaptive", "transnet", "auto"})
DECODE_BACKENDS = frozenset({"opencv", "ffmpeg"})

# Canonical identity flow (research-locked, 2026 OSS practice):
#   detect (RetinaFace / EfficientDet) → quality gates → embed (ArcFace / OML)
#   → strict assign/merge with reject-to-noise (see identity_gates).
# Live + CGI: InsightFace buffalo_l (RetinaFace-10GF + WebFace600K R50 ArcFace).
# 2D cel: AnimeCV (EfficientDet + OML). DeepFace = lab A/B only, never default.
# Identity gate tables live in identity_gates.GATE_PRESETS; stylized presets
# run tighter because embedding space compresses across characters.
CONTENT_PRESETS: dict[str, dict[str, Any]] = {
    "live_action": {
        "label": "Live-action",
        "blurb": "RetinaFace + ArcFace (buffalo_l). Precision-biased merge.",
        "cluster_merge_sim": 0.58,
        "cluster_min_size": 8,
        "min_face_size": 40,
        "det_size": 1280,
        "default_backend": "insightface",
        "default_model": "buffalo_l",
    },
    "animation": {
        "label": "3D / stylized",
        "blurb": "CGI faces. Same buffalo_l stack; stricter merge than live.",
        "cluster_merge_sim": 0.62,
        "cluster_min_size": 6,
        "min_face_size": 40,
        "det_size": 1280,
        "default_backend": "insightface",
        "default_model": "buffalo_l",
    },
    "anime": {
        "label": "Anime / cartoon",
        "blurb": "2D cel. EfficientDet + AnimeCV Cel Canon; strict merge.",
        "cluster_merge_sim": 0.62,
        "cluster_min_size": 5,
        "min_face_size": 36,
        "det_size": 1280,
        "default_backend": "animecv",
        "default_model": "cel_canon",
    },
}

CONTENT_PRESET_KEYS = frozenset(CONTENT_PRESETS.keys())
_PRESET_VALUE_KEYS = (
    "cluster_merge_sim",
    "cluster_min_size",
    "min_face_size",
    "det_size",
    "default_backend",
    "default_model",
)


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_prefix="AUTOSCENE_")

    host: str = "127.0.0.1"
    port: int = 8741
    data_dir: Path = Path.home() / ".autoscene"
    projects_dir: Path | None = None
    default_backend: str = "insightface"
    default_model: str = "buffalo_l"
    sample_fps: float = 6.0
    det_size: int = 1280
    # Longest side (px) before InsightFace. 0 = full native (slow on 4K).
    # 1920 keeps cast quality; cuts host→GPU upload + SCRFD work on UHD.
    detect_max_edge: int = 1920
    min_face_size: int = 40
    cluster_min_size: int = 8
    # Cosine similarity floor for merging split identities (higher = fewer merges).
    cluster_merge_sim: float = 0.55
    # live_action | animation | anime | "" (custom / manual)
    content_preset: str = "animation"
    # Bridge brief miss-detects at typical sample_fps without gluing whole acts.
    segment_gap_sec: float = 2.5
    min_segment_sec: float = 0.5
    # AnimeCV EfficientDet: higher = fewer logo / title-card false faces.
    animecv_score_threshold: float = 0.58
    # Decode thread queue depth — keeps GPU fed while CPU reads frames
    prefetch_depth: int = 8
    # opencv | ffmpeg. OpenCV measured faster than an ffmpeg rawvideo pipe on
    # every Apple Silicon case tried (720p 6.7x, 1080p 1.9x, 4K 1.2x), and
    # VideoToolbox hwaccel was no better than software decode. The pipe is kept
    # for `-hwaccel cuda` on Windows NVIDIA, which was not benchmarked — do not
    # flip this default without measuring on that hardware.
    decode_backend: str = "opencv"
    # Per-frame face selection (geometric; not semantic "background" detection)
    # all | largest | relative | top_n
    face_select_mode: str = "all"
    face_relative_pct: float = 40.0
    face_top_n: int = 2
    # Scene / shot cuts: adaptive | transnet | auto
    scene_cut_detector: str = "auto"
    scene_cut_adaptive_threshold: float = 3.0
    scene_cut_transnet_threshold: float = 0.5

    def ensure_dirs(self) -> None:
        self.data_dir.mkdir(parents=True, exist_ok=True)
        projects = self.projects_dir or (self.data_dir / "projects")
        projects.mkdir(parents=True, exist_ok=True)
        self.projects_dir = projects

    def runtime_path(self) -> Path:
        return self.data_dir / "settings.json"

    def load_runtime_overrides(self) -> None:
        path = self.runtime_path()
        if not path.exists():
            return
        try:
            data = json.loads(path.read_text(encoding="utf-8-sig"))
        except (OSError, json.JSONDecodeError) as exc:
            log.warning("Could not read %s: %s", path, exc)
            return
        if not isinstance(data, dict):
            return
        self.apply_runtime(data, persist=False)

    def _matches_preset(self, preset_id: str) -> bool:
        preset = CONTENT_PRESETS.get(preset_id)
        if not preset:
            return False
        for key in _PRESET_VALUE_KEYS:
            cur = getattr(self, key)
            want = preset[key]
            if key == "cluster_merge_sim":
                if abs(float(cur) - float(want)) > 1e-6:
                    return False
            elif key in ("default_backend", "default_model"):
                if str(cur).strip().lower() != str(want).strip().lower():
                    return False
            elif int(cur) != int(want):
                return False
        return True

    def _sync_content_preset_label(self) -> None:
        """Keep content_preset in sync when knobs are edited manually."""
        if self.content_preset in CONTENT_PRESET_KEYS and self._matches_preset(
            self.content_preset
        ):
            return
        for pid in CONTENT_PRESET_KEYS:
            if self._matches_preset(pid):
                self.content_preset = pid
                return
        self.content_preset = ""

    def apply_runtime(self, patch: dict[str, Any], *, persist: bool = True) -> dict[str, Any]:
        """Update mutable scan knobs. Returns the public settings dict."""
        # Apply named preset first so later keys in the same patch can override.
        if "content_preset" in patch and patch["content_preset"] is not None:
            raw = str(patch["content_preset"]).strip().lower()
            if raw in CONTENT_PRESET_KEYS:
                preset = CONTENT_PRESETS[raw]
                self.content_preset = raw
                self.cluster_merge_sim = float(preset["cluster_merge_sim"])
                self.cluster_min_size = int(preset["cluster_min_size"])
                self.min_face_size = int(preset["min_face_size"])
                self.det_size = int(preset["det_size"])
                self.default_backend = str(preset["default_backend"])
                self.default_model = str(preset["default_model"])
            elif raw in ("", "custom", "none"):
                self.content_preset = ""

        if "sample_fps" in patch and patch["sample_fps"] is not None:
            # Cap 12: higher burns wall-clock on longform with little cast gain.
            self.sample_fps = float(max(1.0, min(12.0, float(patch["sample_fps"]))))
        if "det_size" in patch and patch["det_size"] is not None:
            # Common InsightFace sizes; clamp to sane range
            size = int(patch["det_size"])
            self.det_size = int(max(320, min(1280, size)))
        if "detect_max_edge" in patch and patch["detect_max_edge"] is not None:
            # 0 = native; else clamp to practical UHD-friendly range
            edge = int(patch["detect_max_edge"])
            self.detect_max_edge = 0 if edge <= 0 else int(max(640, min(3840, edge)))
        if "min_face_size" in patch and patch["min_face_size"] is not None:
            self.min_face_size = int(max(16, min(256, int(patch["min_face_size"]))))
        if "cluster_min_size" in patch and patch["cluster_min_size"] is not None:
            self.cluster_min_size = int(max(2, min(64, int(patch["cluster_min_size"]))))
        if "cluster_merge_sim" in patch and patch["cluster_merge_sim"] is not None:
            self.cluster_merge_sim = float(
                max(0.25, min(0.9, float(patch["cluster_merge_sim"])))
            )
        if "prefetch_depth" in patch and patch["prefetch_depth"] is not None:
            self.prefetch_depth = int(max(1, min(32, int(patch["prefetch_depth"]))))
        if "decode_backend" in patch and patch["decode_backend"] is not None:
            raw = str(patch["decode_backend"]).strip().lower()
            if raw in DECODE_BACKENDS:
                self.decode_backend = raw
        if "face_select_mode" in patch and patch["face_select_mode"] is not None:
            mode = str(patch["face_select_mode"]).strip().lower()
            if mode in FACE_SELECT_MODES:
                self.face_select_mode = mode
        if "face_relative_pct" in patch and patch["face_relative_pct"] is not None:
            self.face_relative_pct = float(
                max(1.0, min(100.0, float(patch["face_relative_pct"])))
            )
        if "face_top_n" in patch and patch["face_top_n"] is not None:
            self.face_top_n = int(max(1, min(12, int(patch["face_top_n"]))))
        if "default_backend" in patch and patch["default_backend"]:
            self.default_backend = str(patch["default_backend"])
        if "default_model" in patch and patch["default_model"]:
            self.default_model = str(patch["default_model"])
        if "scene_cut_detector" in patch and patch["scene_cut_detector"] is not None:
            mode = str(patch["scene_cut_detector"]).strip().lower()
            if mode in SCENE_CUT_MODES:
                self.scene_cut_detector = mode
        if (
            "scene_cut_adaptive_threshold" in patch
            and patch["scene_cut_adaptive_threshold"] is not None
        ):
            self.scene_cut_adaptive_threshold = float(
                max(1.0, min(10.0, float(patch["scene_cut_adaptive_threshold"])))
            )
        if (
            "scene_cut_transnet_threshold" in patch
            and patch["scene_cut_transnet_threshold"] is not None
        ):
            self.scene_cut_transnet_threshold = float(
                max(0.1, min(0.95, float(patch["scene_cut_transnet_threshold"])))
            )

        # Manual knob edits clear / retarget the preset badge
        if any(k in patch for k in _PRESET_VALUE_KEYS) and (
            "content_preset" not in patch or patch.get("content_preset") is None
        ):
            self._sync_content_preset_label()

        if persist:
            self.save_runtime_overrides()
        return self.public_dict()

    def save_runtime_overrides(self) -> None:
        self.ensure_dirs()
        payload = {k: getattr(self, k) for k in RUNTIME_SETTING_KEYS}
        # Paths as strings for JSON friendliness if ever added
        path = self.runtime_path()
        path.write_text(json.dumps(payload, indent=2), encoding="utf-8")

    def public_dict(self) -> dict[str, Any]:
        from autoscene.models_vault import ANIMECV_ENCODERS, encoder_ready

        return {
            "sample_fps": self.sample_fps,
            "det_size": self.det_size,
            "detect_max_edge": self.detect_max_edge,
            "min_face_size": self.min_face_size,
            "cluster_min_size": self.cluster_min_size,
            "cluster_merge_sim": self.cluster_merge_sim,
            "content_preset": self.content_preset or "",
            "content_presets": [
                {
                    "id": pid,
                    "label": meta["label"],
                    "blurb": meta["blurb"],
                    "cluster_merge_sim": meta["cluster_merge_sim"],
                    "cluster_min_size": meta["cluster_min_size"],
                    "min_face_size": meta["min_face_size"],
                    "det_size": meta["det_size"],
                    "default_backend": meta["default_backend"],
                    "default_model": meta["default_model"],
                }
                for pid, meta in CONTENT_PRESETS.items()
            ],
            "animecv_models": [
                {
                    "id": m["id"],
                    "label": m["label"],
                    "tagline": m["tagline"],
                    "blurb": m["blurb"],
                    "role": m["role"],
                    "installed": encoder_ready(m["id"]),
                }
                for m in ANIMECV_ENCODERS.values()
            ],
            "prefetch_depth": self.prefetch_depth,
            "decode_backend": self.decode_backend,
            "face_select_mode": self.face_select_mode,
            "face_relative_pct": self.face_relative_pct,
            "face_top_n": self.face_top_n,
            "default_backend": self.default_backend,
            "default_model": self.default_model,
            "scene_cut_detector": self.scene_cut_detector,
            "scene_cut_adaptive_threshold": self.scene_cut_adaptive_threshold,
            "scene_cut_transnet_threshold": self.scene_cut_transnet_threshold,
            "scene_cut_capabilities": {
                "adaptive": _transnet_vault_ready(),  # soft TransNet on CUDA
                "transnet": _transnet_vault_ready(),
                "modes": ["adaptive", "transnet", "auto"],
                "device": "cuda",
            },
            "data_dir": str(self.data_dir),
        }


def _transnet_vault_ready() -> bool:
    """Cheap check — do not import torch/TransNet at settings load."""
    try:
        import importlib.util

        return importlib.util.find_spec("transnetv2_pytorch") is not None
    except Exception:  # noqa: BLE001
        return False


settings = Settings()
settings.ensure_dirs()
settings.load_runtime_overrides()
# Re-apply named preset so model/backend defaults migrate (e.g. antelopev2 → buffalo_l).
if settings.content_preset in CONTENT_PRESET_KEYS:
    settings.apply_runtime({"content_preset": settings.content_preset}, persist=True)
else:
    settings._sync_content_preset_label()
    if not settings.content_preset and settings.default_model == "antelopev2":
        settings.default_model = "buffalo_l"
        settings.save_runtime_overrides()
