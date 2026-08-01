"""Pick best face crop for cast avatars (visibility + sharpness + exposure)."""

from __future__ import annotations

import numpy as np

from autoscene.faces.base import FaceDetection
from autoscene.faces.thumb import thumb_stats


def _clamp01(x: float) -> float:
    return 0.0 if x < 0.0 else 1.0 if x > 1.0 else float(x)


def face_avatar_score(
    det: FaceDetection,
    *,
    anime_soft: bool = False,
    frame_area: float | None = None,
) -> float:
    """
    Higher = better avatar.

    Native face size dominates (enough pixels to look sharp when shown large).
    Sharpness uses calibrated refs from real AutoScene thumbs (CGI/anime often
    land ~5–200 Laplacian var on 256px lettersboxed crops).
    Detector score is a weak tie-breaker only.
    """
    if not det.has_thumb:
        return -1.0

    x1, y1, x2, y2 = det.bbox
    bw = float(max(0, x2 - x1))
    bh = float(max(0, y2 - y1))
    min_side = min(bw, bh)
    area = bw * bh

    # Hard reject micro faces — upscaling them always looks mushy
    min_px = 56.0 if anime_soft else 72.0
    if min_side < min_px:
        return -0.5

    # 0 at min_px, 1 around ~220px face (fills a 256–512 avatar cleanly)
    size_term = _clamp01((min_side - min_px) / (220.0 - min_px))
    if frame_area and frame_area > 0:
        frac = area / float(frame_area)
        size_term = max(size_term, _clamp01(frac / 0.045))

    # Measured at crop time (FaceDetection.with_thumb); never decoded here.
    sharp = float(det.laplacian_var)
    # Empiric: mush ~3–15, ok ~40–120, crisp ~180–350 on our 256 thumbs
    sharp_lo = 8.0 if anime_soft else 18.0
    sharp_hi = 120.0 if anime_soft else 220.0
    if sharp < sharp_lo:
        sharp_term = _clamp01(sharp / sharp_lo) * 0.25
    else:
        sharp_term = 0.25 + 0.75 * _clamp01(
            (sharp - sharp_lo) / max(1.0, sharp_hi - sharp_lo)
        )

    det_raw = float(det.det_score)
    det_term = _clamp01(det_raw / 100.0) if det_raw > 1.0 else _clamp01(det_raw)

    mean = float(det.thumb_mean)
    # Crush near-black / crushed CGI stills
    if mean < 18.0:
        exposure = 0.15
    elif mean < 28.0:
        exposure = 0.45
    elif mean > 245.0:
        exposure = 0.55
    else:
        exposure = 1.0

    if anime_soft:
        w_size, w_sharp, w_det = 0.50, 0.35, 0.15
    else:
        w_size, w_sharp, w_det = 0.42, 0.45, 0.13

    base = w_size * size_term + w_sharp * sharp_term + w_det * det_term
    return base * exposure


def avatar_score_from_bgr(
    bgr: np.ndarray | None,
    *,
    anime_soft: bool = False,
    det_score: float = 1.0,
) -> float:
    """Score an existing thumb image (merge / cast_ops paths)."""
    if bgr is None or bgr.size == 0:
        return -1.0
    h, w = bgr.shape[:2]
    lap, mean = thumb_stats(bgr)
    fake = FaceDetection(
        timestamp_sec=0.0,
        bbox=(0, 0, w, h),
        embedding=np.zeros(1, dtype=np.float32),
        det_score=det_score,
        # Sentinel: face_avatar_score only checks presence, never decodes.
        thumbnail_jpeg=b"\xff",
        laplacian_var=lap,
        thumb_mean=mean,
    )
    return face_avatar_score(fake, anime_soft=anime_soft)


def pick_best_avatar(
    detections: list[FaceDetection],
    *,
    anime_soft: bool = False,
    frame_area: float | None = None,
) -> tuple[FaceDetection | None, float]:
    """Return (best detection, score). Always recomputes full ranking."""
    best: FaceDetection | None = None
    best_score = -1.0
    for det in detections:
        score = face_avatar_score(
            det, anime_soft=anime_soft, frame_area=frame_area
        )
        if score > best_score:
            best_score = score
            best = det
    if best is None and detections:
        # Last resort: largest face with a thumb
        with_thumb = [d for d in detections if d.has_thumb]
        pool = with_thumb or detections
        best = max(
            pool,
            key=lambda d: (d.bbox[2] - d.bbox[0]) * (d.bbox[3] - d.bbox[1]),
        )
        best_score = face_avatar_score(
            best, anime_soft=anime_soft, frame_area=frame_area
        )
    return best, best_score
