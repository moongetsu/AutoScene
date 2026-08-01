from __future__ import annotations

from dataclasses import dataclass, field
from typing import Protocol

import numpy as np


@dataclass
class FaceDetection:
    timestamp_sec: float
    bbox: tuple[int, int, int, int]  # x1,y1,x2,y2
    embedding: np.ndarray
    det_score: float = 1.0
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
    ) -> FaceDetection:
        """Build a detection, cropping and measuring its thumbnail in one pass."""
        from autoscene.faces.thumb import build_thumb

        rec = build_thumb(frame_bgr, bbox)
        return cls(
            timestamp_sec=timestamp_sec,
            bbox=bbox,
            embedding=embedding,
            det_score=det_score,
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
