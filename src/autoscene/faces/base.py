from __future__ import annotations

from dataclasses import dataclass, field
from typing import Protocol

import numpy as np


def pose_weight_from_kps(kps) -> float:
    """
    Frontalness weight in [0.3, 1.0] from 5 detector landmarks.

    SCRFD order: left_eye, right_eye, nose, left_mouth, right_mouth.
    Frontal faces keep the nose on the eye midpoint axis; profile / extreme
    poses drift off it. Profile crops still vote — they just cannot drag a
    centroid the way a clean frontal crop does. No kps (non-landmark
    backends) = full weight.
    """
    if kps is None:
        return 1.0
    try:
        pts = [(float(x), float(y)) for x, y in kps]
    except (TypeError, ValueError):
        return 1.0
    if len(pts) < 3:
        return 1.0
    le, re, nose = pts[0], pts[1], pts[2]
    import math

    eye_dist = math.hypot(re[0] - le[0], re[1] - le[1])
    if eye_dist < 1e-3:
        return 1.0
    ecx = (le[0] + re[0]) / 2.0
    ecy = (le[1] + re[1]) / 2.0
    dx = abs(nose[0] - ecx) / eye_dist
    dy = abs(nose[1] - ecy) / eye_dist
    # Frontal: dx ~ 0-0.15, dy ~ 0.25-0.55. Profile pushes dx toward 0.5+.
    off = max(0.0, dx - 0.18) + max(0.0, abs(dy - 0.40) - 0.28) * 0.5
    w = 1.0 - off * 2.2
    return float(max(0.3, min(1.0, w)))


@dataclass
class FaceDetection:
    timestamp_sec: float
    bbox: tuple[int, int, int, int]  # x1,y1,x2,y2
    embedding: np.ndarray
    det_score: float = 1.0
    # Detector landmarks (5 points, detect-frame space). None when the
    # backend has no landmarks — pose weighting then stays neutral.
    kps: tuple[tuple[float, float], ...] | None = None
    # Frontalness weight (see pose_weight_from_kps). Centroid building and
    # identity merges weight by this so profiles do not pollute templates.
    pose_weight: float = 1.0
    # Letterboxed face crop, JPEG-encoded. A dense scan keeps every detection
    # alive until the final publish; as raw 256x256 BGR that was 196KB each
    # (~3.9GB at 20k hits), so the array is never retained. Call thumb_bgr()
    # for the handful of places that genuinely need pixels.
    thumbnail_jpeg: bytes | None = None
    # Measured once at crop time. Avatar ranking and the quality gate read
    # these instead of re-running Laplacian over every thumb per publish.
    laplacian_var: float = 0.0
    thumb_mean: float = 0.0

    @property
    def has_thumb(self) -> bool:
        return bool(self.thumbnail_jpeg)

    def thumb_bgr(self) -> np.ndarray | None:
        """Decode the stored crop. O(detections) callers must not use this."""
        from autoscene.faces.thumb import decode_thumb

        return decode_thumb(self.thumbnail_jpeg)

    @classmethod
    def with_thumb(
        cls,
        *,
        timestamp_sec: float,
        bbox: tuple[int, int, int, int],
        embedding: np.ndarray,
        det_score: float,
        frame_bgr: np.ndarray,
        kps=None,
    ) -> "FaceDetection":
        """Build a detection, cropping and measuring its thumbnail in one pass."""
        from autoscene.faces.thumb import build_thumb

        rec = build_thumb(frame_bgr, bbox)
        kps_tuple: tuple[tuple[float, float], ...] | None = None
        if kps is not None:
            try:
                kps_tuple = tuple((float(x), float(y)) for x, y in kps)
            except (TypeError, ValueError):
                kps_tuple = None
        return cls(
            timestamp_sec=timestamp_sec,
            bbox=bbox,
            embedding=embedding,
            det_score=det_score,
            kps=kps_tuple,
            pose_weight=pose_weight_from_kps(kps_tuple),
            thumbnail_jpeg=rec.jpeg if rec else None,
            laplacian_var=rec.laplacian_var if rec else 0.0,
            thumb_mean=rec.mean if rec else 0.0,
        )


@dataclass
class ScanFrameResult:
    timestamp_sec: float
    faces: list[FaceDetection] = field(default_factory=list)


class FaceBackend(Protocol):
    name: str
    model_id: str

    def prepare(self) -> None: ...

    def detect(self, frame_bgr: np.ndarray, timestamp_sec: float) -> list[FaceDetection]: ...
