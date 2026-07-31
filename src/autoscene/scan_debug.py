"""Per-scan debug log under ~/.autoscene/logs/scan-<id>.log."""

from __future__ import annotations

import logging
import sys
import time
from pathlib import Path
from typing import Any

from autoscene.config import settings

_log = logging.getLogger("autoscene.scan")


def logs_dir() -> Path:
    d = settings.data_dir / "logs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def scan_log_path(project_id: str) -> Path:
    return logs_dir() / f"scan-{project_id}.log"


class ScanDebugger:
    """File + logger sink for one scan run."""

    def __init__(self, project_id: str) -> None:
        self.project_id = project_id
        self.path = scan_log_path(project_id)
        self.started = time.time()
        self._fh = open(self.path, "a", encoding="utf-8")
        self.info("=" * 72)
        self.info("SCAN_DEBUG_START project=%s", project_id)
        self.info("python=%s", sys.version.replace("\n", " "))
        self.info("executable=%s", sys.executable)
        self.info("log_file=%s", self.path)

    def close(self) -> None:
        try:
            self.info(
                "SCAN_DEBUG_END project=%s elapsed_sec=%.1f",
                self.project_id,
                time.time() - self.started,
            )
            self._fh.close()
        except Exception:  # noqa: BLE001
            pass

    def info(self, msg: str, *args: Any) -> None:
        line = msg % args if args else msg
        stamp = time.strftime("%Y-%m-%d %H:%M:%S")
        text = f"{stamp} | {line}"
        try:
            self._fh.write(text + "\n")
            self._fh.flush()
        except Exception:  # noqa: BLE001
            pass
        _log.info("[%s] %s", self.project_id, line)

    def exception(self, msg: str, *args: Any) -> None:
        line = msg % args if args else msg
        self.info("ERROR %s", line)
        _log.exception("[%s] %s", self.project_id, line)

    def dump_settings(self, *, label: str = "settings") -> None:
        pub = settings.public_dict()
        keys = (
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
        )
        self.info("%s {", label)
        for k in keys:
            self.info("  %s=%r", k, pub.get(k))
        self.info("}")

    def dump_project(self, project: Any) -> None:
        self.info(
            "project id=%s name=%r status=%s backend=%s model=%s video=%s",
            getattr(project, "id", "?"),
            getattr(project, "name", "?"),
            getattr(project, "status", "?"),
            getattr(project, "backend", "?"),
            getattr(project, "model", "?"),
            getattr(project, "video_path", "?"),
        )

    def dump_module_versions(self) -> None:
        mods = (
            "cv2",
            "numpy",
            "insightface",
            "onnxruntime",
            "torch",
            "torchvision",
            "timm",
            "animecv",
        )
        for name in mods:
            try:
                mod = __import__(name)
                ver = getattr(mod, "__version__", None) or getattr(mod, "version", "?")
                path = getattr(mod, "__file__", "?")
                extra = ""
                if name == "torch":
                    import torch

                    extra = (
                        f" cuda_available={torch.cuda.is_available()}"
                        f" cuda={getattr(torch.version, 'cuda', None)}"
                    )
                    if torch.cuda.is_available():
                        extra += f" device0={torch.cuda.get_device_name(0)!r}"
                if name == "onnxruntime":
                    import onnxruntime as ort

                    extra = f" providers={ort.get_available_providers()}"
                self.info("mod %s version=%s path=%s%s", name, ver, path, extra)
            except Exception as exc:  # noqa: BLE001
                self.info("mod %s MISSING (%s)", name, exc)

    def dump_backend(self, backend: Any) -> None:
        self.info(
            "backend class=%s name=%s model_id=%s det_size=%s min_face_size=%s",
            type(backend).__name__,
            getattr(backend, "name", "?"),
            getattr(backend, "model_id", "?"),
            getattr(backend, "det_size", "n/a"),
            getattr(backend, "min_face_size", "n/a"),
        )
        if hasattr(backend, "det_coef"):
            self.info(
                "animecv det_coef=%s score_threshold=%s",
                getattr(backend, "det_coef", None),
                getattr(backend, "score_threshold", None),
            )


def tail_scan_log(project_id: str, max_lines: int = 200) -> dict[str, Any]:
    path = scan_log_path(project_id)
    if not path.exists():
        return {"path": str(path), "exists": False, "lines": []}
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        return {"path": str(path), "exists": True, "error": str(exc), "lines": []}
    lines = text.splitlines()
    return {
        "path": str(path),
        "exists": True,
        "lines": lines[-max_lines:],
        "total_lines": len(lines),
    }
