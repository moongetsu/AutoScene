"""Per-shot visual fingerprints for shot -> scene grouping.

Grouping consecutive shots into a dramatic scene needs a cheap "do these two
shots look like the same place" signal. This builds one from frames the face
scan already decoded: an 8x8 spatial thumbnail plus a coarse hue histogram,
averaged over every sampled frame in the shot.

Deliberately fed from the scan decode loop rather than the TransNetV2 cut pass:
the fallback PySceneDetect path (any Mac without the `transnet` extra) never
produces those 48x27 tensors, and scene grouping has to behave the same either
way.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

log = logging.getLogger(__name__)

# 8x8 BGR spatial mean (192 values) + 16-bin hue histogram.
GRID = 8
HUE_BINS = 16
_SPATIAL_DIM = GRID * GRID * 3
FINGERPRINT_DIM = _SPATIAL_DIM + HUE_BINS

FINGERPRINT_FILE = "shot_fingerprints.npz"


def frame_fingerprint(frame_bgr: np.ndarray) -> np.ndarray:
    """8x8 BGR grid + normalized hue histogram, as one float32 vector."""
    small = cv2.resize(frame_bgr, (GRID, GRID), interpolation=cv2.INTER_AREA)
    spatial = small.astype(np.float32).reshape(-1) / 255.0

    hsv = cv2.cvtColor(small, cv2.COLOR_BGR2HSV)
    hist = cv2.calcHist([hsv], [0], None, [HUE_BINS], [0, 180]).reshape(-1)
    total = float(hist.sum())
    if total > 0:
        hist = hist / total

    return np.concatenate([spatial, hist.astype(np.float32)])


@dataclass
class ShotFingerprints:
    """Running per-shot mean fingerprint, accumulated during the scan."""

    n_shots: int

    def __post_init__(self) -> None:
        self._sums = np.zeros((self.n_shots, FINGERPRINT_DIM), dtype=np.float64)
        self._counts = np.zeros(self.n_shots, dtype=np.int64)

    def accumulate(self, shot_index: int, frame_bgr: np.ndarray) -> None:
        if not (0 <= shot_index < self.n_shots) or frame_bgr is None:
            return
        try:
            self._sums[shot_index] += frame_fingerprint(frame_bgr)
            self._counts[shot_index] += 1
        except cv2.error as exc:
            log.debug("fingerprint skipped shot=%s: %s", shot_index, exc)

    def finalize(self) -> tuple[np.ndarray, np.ndarray]:
        """(fingerprints, counts). Shots never sampled stay all-zero."""
        out = np.zeros_like(self._sums, dtype=np.float32)
        seen = self._counts > 0
        out[seen] = (
            self._sums[seen] / self._counts[seen][:, None]
        ).astype(np.float32)
        return out, self._counts.copy()

    def save(self, project_dir: Path) -> None:
        fps, counts = self.finalize()
        try:
            project_dir.mkdir(parents=True, exist_ok=True)
            np.savez_compressed(
                project_dir / FINGERPRINT_FILE,
                fingerprints=fps,
                counts=counts,
            )
        except OSError as exc:
            # Grouping degrades to cast-only signals; never fail a good scan.
            log.warning("shot fingerprint save failed: %s", exc)


def load_fingerprints(project_dir: Path) -> np.ndarray | None:
    path = Path(project_dir) / FINGERPRINT_FILE
    if not path.is_file():
        return None
    try:
        with np.load(path) as data:
            fps = np.asarray(data["fingerprints"], dtype=np.float32)
        return fps if fps.ndim == 2 and fps.shape[1] == FINGERPRINT_DIM else None
    except (OSError, KeyError, ValueError) as exc:
        log.warning("shot fingerprint load failed: %s", exc)
        return None


def fingerprint_similarity(a: np.ndarray, b: np.ndarray) -> float:
    """
    Cosine similarity in [0, 1]; 0.0 when either shot was never sampled.

    Both halves are non-negative, so cosine cannot go negative here.
    """
    na = float(np.linalg.norm(a))
    nb = float(np.linalg.norm(b))
    if na <= 1e-9 or nb <= 1e-9:
        return 0.0
    return float(np.clip(np.dot(a, b) / (na * nb), 0.0, 1.0))
