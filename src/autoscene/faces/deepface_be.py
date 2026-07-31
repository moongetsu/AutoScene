from __future__ import annotations

import logging

import cv2
import numpy as np

from autoscene.faces.base import FaceDetection
from autoscene.faces.thumb import make_face_thumbnail

log = logging.getLogger(__name__)


class DeepFaceBackend:
    """Optional DeepFace quality A/B (Facenet512 / ArcFace + retinaface)."""

    def __init__(
        self,
        model_name: str = "Facenet512",
        detector_backend: str = "retinaface",
        min_face_size: int = 40,
    ) -> None:
        self.name = "deepface"
        self.model_id = model_name
        self.detector_backend = detector_backend
        self.min_face_size = min_face_size
        self._ready = False

    def prepare(self) -> None:
        try:
            from deepface import DeepFace  # noqa: F401
        except ImportError as exc:
            raise RuntimeError(
                "DeepFace not installed. Open Settings → Packs, install DeepFace, "
                "then restart AutoScene."
            ) from exc
        self._ready = True
        log.info(
            "DeepFace ready model=%s detector=%s",
            self.model_id,
            self.detector_backend,
        )

    def detect(self, frame_bgr: np.ndarray, timestamp_sec: float) -> list[FaceDetection]:
        if not self._ready:
            self.prepare()
        from deepface import DeepFace

        try:
            objs = DeepFace.represent(
                img_path=frame_bgr,
                model_name=self.model_id,
                detector_backend=self.detector_backend,
                enforce_detection=False,
            )
        except Exception as exc:  # noqa: BLE001
            log.debug("DeepFace miss @ %.2fs: %s", timestamp_sec, exc)
            return []

        if not isinstance(objs, list):
            objs = [objs]

        out: list[FaceDetection] = []
        h, w = frame_bgr.shape[:2]
        for obj in objs:
            area = obj.get("facial_area") or {}
            x = int(area.get("x", 0))
            y = int(area.get("y", 0))
            bw = int(area.get("w", 0))
            bh = int(area.get("h", 0))
            if bw < self.min_face_size or bh < self.min_face_size:
                continue
            x1, y1 = max(0, x), max(0, y)
            x2, y2 = min(w - 1, x + bw), min(h - 1, y + bh)
            emb = np.asarray(obj.get("embedding"), dtype=np.float32)
            norm = np.linalg.norm(emb)
            if norm > 0:
                emb = emb / norm
            thumb = make_face_thumbnail(frame_bgr, (x1, y1, x2, y2))
            out.append(
                FaceDetection(
                    timestamp_sec=timestamp_sec,
                    bbox=(x1, y1, x2, y2),
                    embedding=emb,
                    det_score=float(obj.get("face_confidence", 1.0) or 1.0),
                    thumbnail_bgr=thumb,
                )
            )
        return out
