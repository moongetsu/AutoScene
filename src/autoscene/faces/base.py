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
    thumbnail_bgr: np.ndarray | None = None


@dataclass
class ScanFrameResult:
    timestamp_sec: float
    faces: list[FaceDetection] = field(default_factory=list)


class FaceBackend(Protocol):
    name: str
    model_id: str

    def prepare(self) -> None: ...

    def detect(self, frame_bgr: np.ndarray, timestamp_sec: float) -> list[FaceDetection]: ...
