"""Face thumbnail helpers — never stretch non-square crops."""

from __future__ import annotations

from dataclasses import dataclass

import cv2
import numpy as np

# JPEG quality for stored face crops. 92 is visually indistinguishable from the
# raw crop at 256px and lands ~10KB vs 196KB for the BGR array.
_THUMB_JPEG_QUALITY = 92


def make_face_thumbnail(
    frame_bgr: np.ndarray,
    bbox: tuple[int, int, int, int],
    *,
    size: int = 256,
    pad_scale: float = 1.35,
) -> np.ndarray | None:
    """
    Expand bbox slightly, crop, letterbox onto a square canvas.
    Preserves aspect — no cv2.resize stretch to square.
    Uses AREA when downscaling, CUBIC when upscaling.
    """
    if frame_bgr is None or frame_bgr.size == 0:
        return None
    h, w = frame_bgr.shape[:2]
    x1, y1, x2, y2 = [int(v) for v in bbox]
    x1, y1 = max(0, x1), max(0, y1)
    x2, y2 = min(w, x2), min(h, y2)
    bw, bh = x2 - x1, y2 - y1
    if bw < 2 or bh < 2:
        return None

    cx = (x1 + x2) / 2.0
    cy = (y1 + y2) / 2.0
    side = max(bw, bh) * float(pad_scale)
    half = side / 2.0
    nx1 = int(max(0, cx - half))
    ny1 = int(max(0, cy - half))
    nx2 = int(min(w, cx + half))
    ny2 = int(min(h, cy + half))
    crop = frame_bgr[ny1:ny2, nx1:nx2]
    if crop.size == 0:
        return None

    ch, cw = crop.shape[:2]
    scale = float(size) / float(max(ch, cw))
    nw = max(1, int(round(cw * scale)))
    nh = max(1, int(round(ch * scale)))
    interp = cv2.INTER_AREA if scale < 0.999 else cv2.INTER_CUBIC
    resized = cv2.resize(crop, (nw, nh), interpolation=interp)

    canvas = np.zeros((size, size, 3), dtype=np.uint8)
    # Soft panel fill (void-ish) so letterbox is not harsh black
    canvas[:] = (18, 16, 22)
    ox = (size - nw) // 2
    oy = (size - nh) // 2
    canvas[oy : oy + nh, ox : ox + nw] = resized
    return canvas


def thumb_stats(bgr: np.ndarray | None) -> tuple[float, float]:
    """(laplacian variance, mean intensity) for a BGR crop; (0, 0) when empty."""
    if bgr is None or bgr.size == 0:
        return 0.0, 0.0
    gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY) if bgr.ndim == 3 else bgr
    return float(cv2.Laplacian(gray, cv2.CV_64F).var()), float(np.mean(bgr))


def encode_thumb(bgr: np.ndarray | None) -> bytes | None:
    if bgr is None or bgr.size == 0:
        return None
    ok, buf = cv2.imencode(
        ".jpg", bgr, [int(cv2.IMWRITE_JPEG_QUALITY), _THUMB_JPEG_QUALITY]
    )
    return buf.tobytes() if ok else None


def decode_thumb(data: bytes | None) -> np.ndarray | None:
    if not data:
        return None
    img = cv2.imdecode(np.frombuffer(data, np.uint8), cv2.IMREAD_COLOR)
    return img if img is not None and img.size else None


@dataclass(frozen=True)
class ThumbRecord:
    """Encoded face crop plus the stats measured while it was still decoded."""

    jpeg: bytes
    laplacian_var: float
    mean: float


def build_thumb(
    frame_bgr: np.ndarray,
    bbox: tuple[int, int, int, int],
    *,
    size: int = 256,
    pad_scale: float = 1.35,
) -> ThumbRecord | None:
    """
    Crop, measure, encode — in one pass over the pixels.

    Detections are held in memory for the whole scan, so they store the JPEG
    rather than the array. Sharpness/exposure are measured here because the
    avatar ranking would otherwise have to decode every thumbnail again on
    every live-cluster publish.
    """
    crop = make_face_thumbnail(frame_bgr, bbox, size=size, pad_scale=pad_scale)
    if crop is None:
        return None
    jpeg = encode_thumb(crop)
    if jpeg is None:
        return None
    lap, mean = thumb_stats(crop)
    return ThumbRecord(jpeg=jpeg, laplacian_var=lap, mean=mean)


def square_letterbox(bgr: np.ndarray, size: int = 256) -> np.ndarray | None:
    """Letterbox an existing crop to square (for already-extracted thumbs)."""
    if bgr is None or bgr.size == 0:
        return None
    h, w = bgr.shape[:2]
    return make_face_thumbnail(bgr, (0, 0, w, h), size=size, pad_scale=1.0)


def grab_frame_near(video_path, timestamp_sec: float) -> np.ndarray | None:
    """Decode one BGR frame near timestamp (best-effort seek)."""
    from pathlib import Path

    path = Path(video_path)
    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        return None
    try:
        cap.set(cv2.CAP_PROP_POS_MSEC, max(0.0, float(timestamp_sec) * 1000.0))
        ok, frame = cap.read()
        if ok and frame is not None and frame.size:
            return frame
        fps = float(cap.get(cv2.CAP_PROP_FPS) or 0.0)
        if fps > 1.0:
            cap.set(cv2.CAP_PROP_POS_FRAMES, max(0, int(timestamp_sec * fps)))
            ok, frame = cap.read()
            if ok and frame is not None and frame.size:
                return frame
    finally:
        cap.release()
    return None


def render_hq_avatar(
    video_path,
    timestamp_sec: float,
    bbox: tuple[int, int, int, int],
    *,
    size: int = 512,
    pad_scale: float = 1.25,
    fallback_bgr: np.ndarray | None = None,
) -> np.ndarray | None:
    """
    Re-crop avatar from source video at native resolution, then letterbox to size.
    Falls back to existing thumb if seek/crop fails.
    """
    frame = grab_frame_near(video_path, timestamp_sec)
    if frame is not None:
        hq = make_face_thumbnail(frame, bbox, size=size, pad_scale=pad_scale)
        if hq is not None:
            return hq
    if fallback_bgr is not None and fallback_bgr.size:
        return make_face_thumbnail(
            fallback_bgr,
            (0, 0, fallback_bgr.shape[1], fallback_bgr.shape[0]),
            size=size,
            pad_scale=1.0,
        )
    return None
