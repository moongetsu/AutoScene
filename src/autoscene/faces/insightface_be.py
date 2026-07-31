from __future__ import annotations

import logging
import shutil
from pathlib import Path

import cv2
import numpy as np

from autoscene.faces.base import FaceDetection
from autoscene.faces.thumb import make_face_thumbnail
from autoscene.gpu import (
    cuda_provider_options,
    ensure_cuda_dll_path,
    is_cudnn_conv_failure,
    require_cuda_providers,
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

    def prepare(self) -> None:
        from insightface.app import FaceAnalysis
        import insightface.model_zoo.model_zoo as model_zoo

        ensure_insightface_pack(self.model_id)
        ensure_cuda_dll_path()
        require_cuda_providers()  # validate GPU before loading packs
        opts = cuda_provider_options()
        # InsightFace passes providers + provider_options separately into ORT
        model_zoo.get_default_providers = lambda: ["CUDAExecutionProvider"]
        model_zoo.get_default_provider_options = lambda: [opts]

        log.info(
            "InsightFace prepare model=%s det_size=%s modules=%s opts=%s",
            self.model_id,
            self.det_size,
            self.ALLOWED_MODULES,
            opts,
        )
        try:
            self._app = FaceAnalysis(
                name=self.model_id,
                allowed_modules=list(self.ALLOWED_MODULES),
                providers=["CUDAExecutionProvider"],
                provider_options=[opts],
            )
        except AssertionError as exc:
            raise RuntimeError(
                f"InsightFace pack '{self.model_id}' incomplete (no detection model). "
                f"Check ~/.insightface/models/{self.model_id}/*.onnx"
            ) from exc
        # ctx_id must be >=0 so InsightFace does not force CPU
        self._app.prepare(ctx_id=0, det_size=(self.det_size, self.det_size))
        self._assert_cuda_sessions()

    def _assert_cuda_sessions(self) -> None:
        assert self._app is not None
        for task, model in self._app.models.items():
            session = getattr(model, "session", None)
            if session is None:
                continue
            active = session.get_providers()
            if not active or active[0] != "CUDAExecutionProvider":
                raise RuntimeError(
                    f"Model '{task}' not on CUDA (providers={active}). CPU forbidden."
                )
            log.info("GPU ok task=%s providers=%s", task, active)

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
            thumb = make_face_thumbnail(frame_bgr, (x1, y1, x2, y2))
            out.append(
                FaceDetection(
                    timestamp_sec=timestamp_sec,
                    bbox=(x1, y1, x2, y2),
                    embedding=emb,
                    det_score=float(getattr(face, "det_score", 1.0)),
                    thumbnail_bgr=thumb,
                )
            )
        return out
