from __future__ import annotations

import logging

from autoscene.faces.base import FaceBackend
from autoscene.faces.deepface_be import DeepFaceBackend
from autoscene.faces.insightface_be import InsightFaceBackend

log = logging.getLogger(__name__)


def create_backend(
    backend: str = "insightface",
    model: str = "buffalo_l",
    det_size: int = 1280,
    min_face_size: int = 40,
) -> FaceBackend:
    kind = (backend or "insightface").strip().lower()
    log.info(
        "create_backend kind=%r model=%r det_size=%s min_face_size=%s",
        kind,
        model,
        det_size,
        min_face_size,
    )
    if kind == "deepface":
        be = DeepFaceBackend(model_name=model, min_face_size=min_face_size)
        log.info("create_backend → DeepFaceBackend model_id=%s", be.model_id)
        return be
    if kind in ("animecv", "anime"):
        from autoscene.config import settings
        from autoscene.faces.animecv_be import AnimeCVBackend

        be = AnimeCVBackend(
            model_name=model,
            min_face_size=min_face_size,
            score_threshold=float(settings.animecv_score_threshold),
        )
        log.info(
            "create_backend → AnimeCVBackend model_id=%s score=%s (requested=%r)",
            be.model_id,
            be.score_threshold,
            model,
        )
        return be
    be = InsightFaceBackend(
        model_name=model,
        det_size=det_size,
        min_face_size=min_face_size,
    )
    log.info(
        "create_backend → InsightFaceBackend model_id=%s det_size=%s",
        be.model_id,
        be.det_size,
    )
    return be
