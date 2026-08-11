from __future__ import annotations

import logging
import shutil
from pathlib import Path
from typing import Any

import cv2
import numpy as np

from autoscene.faces.base import FaceDetection
from autoscene.gpu import (
    ensure_cuda_dll_path,
    is_cudnn_conv_failure,
    require_providers,
)

log = logging.getLogger(__name__)


def ensure_insightface_pack(model_name: str, root: Path | None = None) -> Path:
    """Ensure ~/.insightface/models/<name>/*.onnx (download + flatten nested zips)."""
    root = root or (Path.home() / ".insightface")
    model_dir = root / "models" / model_name

    def _onnx_here() -> list[Path]:
        return list(model_dir.glob("*.onnx")) if model_dir.is_dir() else []

    def _flatten_nested() -> None:
        # Common zip mistake: models/buffalo_l/buffalo_l/*.onnx
        nested = model_dir / model_name
        if not nested.is_dir():
            return
        for src in nested.glob("*.onnx"):
            dest = model_dir / src.name
            if not dest.exists():
                shutil.move(str(src), str(dest))
                log.info("Moved nested model %s -> %s", src.name, dest)
        try:
            if nested.exists() and not any(nested.iterdir()):
                nested.rmdir()
        except OSError:
            pass

    if _onnx_here():
        return model_dir

    _flatten_nested()
    if _onnx_here():
        return model_dir

    # Empty dir fools insightface download() into a no-op — remove then force fetch.
    if model_dir.is_dir() and not _onnx_here():
        log.warning("Removing empty InsightFace dir before download: %s", model_dir)
        shutil.rmtree(model_dir, ignore_errors=True)

    from insightface.utils.storage import download as insightface_download

    log.info("Downloading InsightFace pack %s into %s", model_name, root)
    insightface_download("models", model_name, force=True, root=str(root))
    _flatten_nested()

    onnx_here = _onnx_here()
    if not onnx_here:
        raise FileNotFoundError(
            f"InsightFace pack '{model_name}' has no .onnx in {model_dir} after download. "
            f"Check network access to github.com/deepinsight/insightface releases."
        )
    return model_dir


def _freeze_detector_shapes(model_dir: Path, model_name: str, det_size: int) -> Path:
    """
    Build a sibling pack whose detector has a bounded input shape.

    SCRFD ships as [1, 3, '?', '?']. CoreML's MLProgram backend cannot compile
    unbounded dims, so it splits the graph and falls back to CPU — measured at
    45ms/frame vs 15.6ms once the spatial dims are pinned (2.9x on the dense
    scan hot path). InsightFace pins the runtime size via prepare(det_size)
    anyway, so pinning it in the graph loses nothing.

    Returns the pack name to hand to FaceAnalysis.
    """
    import onnx

    out_dir = model_dir.parent / f"{model_name}_static{det_size}"
    marker = out_dir / ".autoscene_static"
    if marker.is_file() and list(out_dir.glob("*.onnx")):
        return out_dir

    log.info("Building static-shape InsightFace pack for CoreML: %s", out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    frozen = 0
    kept = 0
    for src in sorted(model_dir.glob("*.onnx")):
        dest = out_dir / src.name
        try:
            model = onnx.load(str(src))
            inp = model.graph.input[0]
            dims = list(inp.type.tensor_type.shape.dim)

            def _dyn(d: Any) -> bool:
                return d.HasField("dim_param") or d.dim_value <= 0

            # Classify by input shape, not filename — pack layouts differ
            # (buffalo_l det_10g vs antelopev2 scrfd_10g_bnkps).
            is_detector = len(dims) == 4 and all(_dyn(d) for d in dims[2:4])
            is_recognition = (
                len(dims) == 4
                and not _dyn(dims[2])
                and dims[2].dim_value == 112
                and dims[3].dim_value == 112
            )
            if not (is_detector or is_recognition):
                # genderage / landmark nets: ALLOWED_MODULES drops them anyway,
                # but InsightFace still builds a session per file it finds, and
                # each unbounded one prints CoreML E5RT noise. Skip them.
                log.debug("Skipping unused model %s", src.name)
                continue

            if is_recognition:
                shutil.copy2(src, dest)
                kept += 1
                continue

            for d, v in zip(dims, [1, 3, det_size, det_size]):
                d.ClearField("dim_param")
                d.dim_value = v
            # Downstream outputs inherit stale symbolic dims; let ORT re-infer.
            for out in model.graph.output:
                for d in out.type.tensor_type.shape.dim:
                    if d.HasField("dim_param"):
                        d.ClearField("dim_param")
                        d.dim_value = 0
            onnx.save(model, str(dest))
            frozen += 1
            kept += 1
            log.info("Froze %s to 1x3x%sx%s", src.name, det_size, det_size)
        except Exception as exc:  # noqa: BLE001
            log.warning("Could not freeze %s (%s) — copying as-is", src.name, exc)
            shutil.copy2(src, dest)
            kept += 1

    if not frozen or kept < 2:
        # Nothing to gain; drop the copy so we do not shadow the real pack.
        shutil.rmtree(out_dir, ignore_errors=True)
        return model_dir

    marker.write_text("autoscene static-shape pack\n", encoding="utf-8")
    return out_dir


class InsightFaceBackend:
    """Quality-first InsightFace pack (antelopev2 / buffalo_l).

    Loads detection + recognition only (no genderage / landmark nets).
    Cast ID needs embed; side heads burn GPU for no cluster value.
    """

    # Cast pipeline only needs these InsightFace tasknames
    ALLOWED_MODULES = ("detection", "recognition")

    def __init__(
        self,
        model_name: str = "buffalo_l",
        det_size: int = 1280,
        min_face_size: int = 40,
    ) -> None:
        self.name = "insightface"
        self.model_id = model_name
        self.det_size = det_size
        self.min_face_size = min_face_size
        self._app = None
        self._cudnn_fallback_done = False
        self._provider = ""

    def prepare(self) -> None:
        from insightface.app import FaceAnalysis
        import insightface.model_zoo.model_zoo as model_zoo

        model_dir = ensure_insightface_pack(self.model_id)
        ensure_cuda_dll_path()
        # Validates the platform's provider before loading packs. On macOS this
        # resolves to CoreML (or CPU if CoreML cannot build a session).
        providers = require_providers()
        names = [name for name, _ in providers]
        opts = [dict(o) for _, o in providers]
        self._provider = names[0]
        # InsightFace passes providers + provider_options separately into ORT
        model_zoo.get_default_providers = lambda: list(names)
        model_zoo.get_default_provider_options = lambda: [dict(o) for o in opts]

        pack_name = self.model_id
        if self._provider == "CoreMLExecutionProvider":
            try:
                static_dir = _freeze_detector_shapes(
                    model_dir, self.model_id, self.det_size
                )
                pack_name = static_dir.name
            except Exception as exc:  # noqa: BLE001
                # Not worth failing the load over — the dynamic pack still runs.
                log.warning("Static-shape pack unavailable (%s); using stock pack", exc)

        log.info(
            "InsightFace prepare model=%s pack=%s det_size=%s modules=%s providers=%s opts=%s",
            self.model_id,
            pack_name,
            self.det_size,
            self.ALLOWED_MODULES,
            names,
            opts,
        )
        try:
            self._app = FaceAnalysis(
                name=pack_name,
                allowed_modules=list(self.ALLOWED_MODULES),
                providers=list(names),
                provider_options=[dict(o) for o in opts],
            )
        except AssertionError as exc:
            raise RuntimeError(
                f"InsightFace pack '{self.model_id}' incomplete (no detection model). "
                f"Check ~/.insightface/models/{self.model_id}/*.onnx"
            ) from exc
        # ctx_id must be >=0 so InsightFace does not force CPU. On macOS there is
        # no CUDA device to index, but a negative id makes InsightFace rewrite the
        # provider list to CPU-only and discard CoreML.
        self._app.prepare(ctx_id=0, det_size=(self.det_size, self.det_size))
        self._assert_sessions()

    def _assert_sessions(self) -> None:
        """Every loaded model must sit on the provider we asked for."""
        assert self._app is not None
        expected = self._provider
        for task, model in self._app.models.items():
            session = getattr(model, "session", None)
            if session is None:
                continue
            active = session.get_providers()
            if not active or active[0] != expected:
                raise RuntimeError(
                    f"Model '{task}' not on {expected} (providers={active})."
                )
            log.info("accel ok task=%s providers=%s", task, active)

    # Back-compat alias
    def _assert_cuda_sessions(self) -> None:
        self._assert_sessions()

    def _rebuild_smaller_det(self) -> None:
        fallback = min(self.det_size, 640)
        log.warning(
            "cuDNN Conv failure — rebuilding InsightFace with det_size=%s",
            fallback,
        )
        self.det_size = fallback
        self._app = None
        self.prepare()

    def detect(self, frame_bgr: np.ndarray, timestamp_sec: float) -> list[FaceDetection]:
        if self._app is None:
            self.prepare()
        assert self._app is not None

        try:
            faces = self._app.get(frame_bgr)
        except Exception as exc:  # noqa: BLE001
            if is_cudnn_conv_failure(exc) and not self._cudnn_fallback_done:
                self._cudnn_fallback_done = True
                self._rebuild_smaller_det()
                assert self._app is not None
                faces = self._app.get(frame_bgr)
            else:
                raise

        out: list[FaceDetection] = []
        h, w = frame_bgr.shape[:2]
        for face in faces:
            x1, y1, x2, y2 = [int(v) for v in face.bbox]
            x1, y1 = max(0, x1), max(0, y1)
            x2, y2 = min(w - 1, x2), min(h - 1, y2)
            bw, bh = x2 - x1, y2 - y1
            if bw < self.min_face_size or bh < self.min_face_size:
                continue
            emb = np.asarray(face.embedding, dtype=np.float32)
            norm = np.linalg.norm(emb)
            if norm > 0:
                emb = emb / norm
            kps = getattr(face, "kps", None)
            out.append(
                FaceDetection.with_thumb(
                    timestamp_sec=timestamp_sec,
                    bbox=(x1, y1, x2, y2),
                    embedding=emb,
                    det_score=float(getattr(face, "det_score", 1.0)),
                    frame_bgr=frame_bgr,
                    kps=kps,
                )
            )
        return out
