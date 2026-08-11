"""Cached JPEG stills for Export scene tiles.

Focus-person face crops are preferred when detection geometry is known so a
tile never shows the opposing reverse-shot plate of someone else.
"""

from __future__ import annotations

import logging
import subprocess
import threading
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import cv2
import numpy as np

from autoscene.export import ensure_ffmpeg

log = logging.getLogger(__name__)

# Bump when thumb recipe changes so old wrong-plate caches are ignored.
_FRAME_DIR = "scene_frames_v4"
_FACE_DIR = "face_crops_v4"
_locks: dict[str, threading.Lock] = {}
_locks_guard = threading.Lock()
# Parallel extracts — proxy H.264 + CUDA decode handle more concurrency.
_extract_sem = threading.Semaphore(8)

# Hot JPEG bytes in RAM (path → bytes). Evict LRU past budget.
_ram_guard = threading.Lock()
_ram_jpegs: OrderedDict[str, bytes] = OrderedDict()
_RAM_BUDGET = 512 * 1024 * 1024  # 512 MiB
_ram_bytes = 0

_prefetch_guard = threading.Lock()
_prefetch_running: set[str] = set()


def _lock_for(key: str) -> threading.Lock:
    with _locks_guard:
        lock = _locks.get(key)
        if lock is None:
            lock = threading.Lock()
            _locks[key] = lock
        return lock


def frame_cache_path(cache_dir: Path, time_sec: float) -> Path:
    ms = max(0, int(round(time_sec * 1000)))
    return cache_dir / _FRAME_DIR / f"{ms:08d}.jpg"


def face_crop_cache_path(
    cache_dir: Path, person_id: int, time_sec: float
) -> Path:
    ms = max(0, int(round(time_sec * 1000)))
    return cache_dir / _FACE_DIR / f"p{int(person_id)}_{ms:08d}.jpg"


def ram_get_jpeg(path: Path) -> bytes | None:
    key = str(path.resolve())
    with _ram_guard:
        data = _ram_jpegs.get(key)
        if data is None:
            return None
        _ram_jpegs.move_to_end(key)
        return data


def ram_put_jpeg(path: Path, data: bytes) -> None:
    if not data or len(data) < 200:
        return
    key = str(path.resolve())
    global _ram_bytes
    with _ram_guard:
        old = _ram_jpegs.pop(key, None)
        if old is not None:
            _ram_bytes -= len(old)
        _ram_jpegs[key] = data
        _ram_bytes += len(data)
        while _ram_bytes > _RAM_BUDGET and _ram_jpegs:
            _, evicted = _ram_jpegs.popitem(last=False)
            _ram_bytes -= len(evicted)


def read_scene_frame_bytes(path: Path) -> bytes:
    """Disk → RAM promote for hot Export tiles."""
    hit = ram_get_jpeg(path)
    if hit is not None:
        return hit
    data = path.read_bytes()
    ram_put_jpeg(path, data)
    return data


def _hw_input_args() -> list[str]:
    try:
        from autoscene.ffmpeg_encode import hwaccel_input_args

        return list(hwaccel_input_args(zero_copy=False))
    except Exception:  # noqa: BLE001
        return []


def _run_extract(ffmpeg: str, src: Path, t: float, out: Path) -> bool:
    """Fast input seek first; accurate / GPU-decode fallbacks if needed."""
    vf = "scale='min(640,iw)':-2,eq=brightness=0.06:contrast=1.12"
    hw = _hw_input_args()
    attempts: list[list[str]] = [
        # Fast: keyframe seek before open (best on H.264 proxy)
        [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            f"{t:.3f}",
            "-i",
            str(src),
            "-frames:v",
            "1",
            "-q:v",
            "3",
            "-vf",
            vf,
            str(out),
        ],
    ]
    if hw:
        # GPU decode + accurate seek (helps HEVC Main10 source before proxy ready)
        attempts.append(
            [
                ffmpeg,
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                *hw,
                "-ss",
                f"{t:.3f}",
                "-i",
                str(src),
                "-frames:v",
                "1",
                "-q:v",
                "3",
                "-vf",
                vf,
                str(out),
            ]
        )
    attempts.append(
        # Accurate CPU: decode then seek
        [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            str(src),
            "-ss",
            f"{t:.3f}",
            "-frames:v",
            "1",
            "-q:v",
            "3",
            "-vf",
            vf,
            str(out),
        ]
    )
    last_err = ""
    for cmd in attempts:
        try:
            out.unlink(missing_ok=True)
        except OSError:
            pass
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            timeout=45,
        )
        if proc.returncode == 0 and out.is_file() and out.stat().st_size > 200:
            try:
                ram_put_jpeg(out, out.read_bytes())
            except OSError:
                pass
            return True
        last_err = (proc.stderr or "")[-300:].strip()
    if last_err:
        log.warning("scene frame ffmpeg fail t=%.3f src=%s: %s", t, src.name, last_err)
    return False


def lookup_face_bbox(
    cache_dir: Path,
    person_id: int,
    time_sec: float,
    *,
    max_dt: float = 0.75,
) -> tuple[float, tuple[int, int, int, int]] | None:
    """
    Nearest detection of ``person_id`` to ``time_sec`` from detection_cache.npz.
    Returns (matched_time, bbox) or None when geometry/labels are missing.
    """
    from autoscene.identity_adapter import load_detection_cache

    cache = load_detection_cache(cache_dir)
    if cache is None:
        return None
    bboxes = cache.get("bboxes")
    labels = cache.get("labels")
    timestamps = cache.get("timestamps")
    if bboxes is None or labels is None or timestamps is None:
        return None
    labels = np.asarray(labels).reshape(-1)
    timestamps = np.asarray(timestamps, dtype=np.float64).reshape(-1)
    bb = np.asarray(bboxes, dtype=np.float64).reshape(-1, 4)
    n = min(labels.size, timestamps.size, bb.shape[0])
    if n == 0:
        return None
    mask = labels[:n] == int(person_id)
    if not np.any(mask):
        return None
    idxs = np.where(mask)[0]
    dts = np.abs(timestamps[idxs] - float(time_sec))
    best_i = int(idxs[int(np.argmin(dts))])
    if float(dts.min()) > float(max_dt):
        return None
    row = bb[best_i]
    bbox = (int(row[0]), int(row[1]), int(row[2]), int(row[3]))
    if bbox[2] - bbox[0] < 2 or bbox[3] - bbox[1] < 2:
        return None
    return float(timestamps[best_i]), bbox


def ensure_face_crop_frame(
    video_path: Path,
    cache_dir: Path,
    time_sec: float,
    person_id: int,
    *,
    prefer_proxy: bool = True,
    size: int = 512,
) -> Path | None:
    """
    Focus-person face crop JPEG, or None if cache/geometry unavailable.
    Falls back is the caller's job (full-plate ensure_scene_frame).
    """
    hit = lookup_face_bbox(cache_dir, person_id, time_sec)
    if hit is None:
        return None
    matched_t, bbox = hit
    out = face_crop_cache_path(cache_dir, person_id, matched_t)
    out.parent.mkdir(parents=True, exist_ok=True)
    if out.is_file() and out.stat().st_size > 200:
        try:
            ram_put_jpeg(out, out.read_bytes())
        except OSError:
            pass
        return out

    lock = _lock_for(str(out.resolve()))
    with lock:
        if out.is_file() and out.stat().st_size > 200:
            try:
                ram_put_jpeg(out, out.read_bytes())
            except OSError:
                pass
            return out

        from autoscene.faces.thumb import render_hq_avatar

        src = video_path.resolve()
        candidates: list[Path] = []
        if prefer_proxy:
            proxy = cache_dir / "browser_proxy.mp4"
            if proxy.is_file() and proxy.stat().st_size > 1024:
                candidates.append(proxy)
        if src.is_file():
            candidates.append(src)
        if not candidates:
            return None

        crop = None
        for src_try in candidates:
            crop = render_hq_avatar(
                src_try,
                matched_t,
                bbox,
                size=size,
                pad_scale=1.25,
            )
            if crop is not None:
                break
        if crop is None:
            return None
        ok = cv2.imwrite(
            str(out),
            crop,
            [int(cv2.IMWRITE_JPEG_QUALITY), 90],
        )
        if ok and out.is_file() and out.stat().st_size > 200:
            try:
                ram_put_jpeg(out, out.read_bytes())
            except OSError:
                pass
            return out
        return None


def ensure_scene_frame(
    video_path: Path,
    cache_dir: Path,
    time_sec: float,
    *,
    prefer_proxy: bool = True,
    person_id: int | None = None,
) -> Path:
    """
    JPEG near time_sec. When person_id is set and detection geometry exists,
    return a focus-person face crop; otherwise full plate.
    """
    if person_id is not None:
        try:
            face = ensure_face_crop_frame(
                video_path,
                cache_dir,
                time_sec,
                int(person_id),
                prefer_proxy=prefer_proxy,
            )
            if face is not None:
                return face
        except Exception as exc:  # noqa: BLE001
            log.warning(
                "face crop fallback to plate person=%s t=%.3f: %s",
                person_id,
                time_sec,
                exc,
            )

    t = max(0.0, float(time_sec))
    out = frame_cache_path(cache_dir, t)
    out.parent.mkdir(parents=True, exist_ok=True)

    if out.is_file() and out.stat().st_size > 200:
        try:
            ram_put_jpeg(out, out.read_bytes())
        except OSError:
            pass
        return out

    lock = _lock_for(str(out.resolve()))
    with lock:
        if out.is_file() and out.stat().st_size > 200:
            try:
                ram_put_jpeg(out, out.read_bytes())
            except OSError:
                pass
            return out

        candidates: list[Path] = []
        if prefer_proxy:
            proxy = cache_dir / "browser_proxy.mp4"
            if proxy.is_file() and proxy.stat().st_size > 1024:
                candidates.append(proxy)
        src = video_path.resolve()
        if src.is_file():
            candidates.append(src)
        if not candidates:
            raise FileNotFoundError(str(video_path))

        ffmpeg = ensure_ffmpeg()
        with _extract_sem:
            if out.is_file() and out.stat().st_size > 200:
                return out
            for src_try in candidates:
                try:
                    if _run_extract(ffmpeg, src_try, t, out):
                        return out
                except subprocess.TimeoutExpired:
                    log.warning("scene frame timeout t=%.3f src=%s", t, src_try.name)
                    try:
                        out.unlink(missing_ok=True)
                    except OSError:
                        pass

        raise RuntimeError(f"Could not extract frame at {t:.3f}s")


def prefetch_scene_frames(
    video_path: Path,
    cache_dir: Path,
    times_sec: list[float],
    *,
    prefer_proxy: bool = True,
    max_workers: int = 6,
) -> dict[str, int]:
    """
    Extract missing Export stills in parallel (GPU decode when available).
    Idempotent; skips times already on disk / in RAM.
    """
    uniq: list[float] = []
    seen: set[int] = set()
    for raw in times_sec:
        t = max(0.0, float(raw))
        ms = int(round(t * 1000))
        if ms in seen:
            continue
        seen.add(ms)
        uniq.append(t)

    ready = 0
    todo: list[float] = []
    for t in uniq:
        out = frame_cache_path(cache_dir, t)
        if ram_get_jpeg(out) is not None or (
            out.is_file() and out.stat().st_size > 200
        ):
            if out.is_file():
                try:
                    ram_put_jpeg(out, out.read_bytes())
                except OSError:
                    pass
            ready += 1
        else:
            todo.append(t)

    if not todo:
        return {"total": len(uniq), "ready": ready, "queued": 0, "built": 0}

    built = 0
    errors = 0

    def _one(t: float) -> bool:
        try:
            ensure_scene_frame(
                video_path, cache_dir, t, prefer_proxy=prefer_proxy, person_id=None
            )
            return True
        except Exception as exc:  # noqa: BLE001
            log.warning("prefetch frame fail t=%.3f: %s", t, exc)
            return False

    workers = max(1, min(int(max_workers), len(todo)))
    with ThreadPoolExecutor(max_workers=workers) as pool:
        for ok in pool.map(_one, todo):
            if ok:
                built += 1
            else:
                errors += 1

    return {
        "total": len(uniq),
        "ready": ready + built,
        "queued": len(todo),
        "built": built,
        "errors": errors,
    }


def start_prefetch_scene_frames(
    video_path: Path,
    cache_dir: Path,
    times_sec: list[float],
    *,
    prefer_proxy: bool = True,
) -> dict[str, object]:
    """Kick a background prefetch once per project cache dir."""
    key = str(cache_dir.resolve())
    with _prefetch_guard:
        if key in _prefetch_running:
            return {"started": False, "building": True, "reason": "already_running"}
        _prefetch_running.add(key)

    def _run() -> None:
        try:
            stats = prefetch_scene_frames(
                video_path,
                cache_dir,
                times_sec,
                prefer_proxy=prefer_proxy,
            )
            log.info(
                "scene frame prefetch done dir=%s total=%s ready=%s built=%s",
                cache_dir.name,
                stats.get("total"),
                stats.get("ready"),
                stats.get("built"),
            )
        finally:
            with _prefetch_guard:
                _prefetch_running.discard(key)

    threading.Thread(
        target=_run, daemon=True, name=f"scene-frame-prefetch-{cache_dir.name}"
    ).start()
    return {"started": True, "building": True, "count": len(times_sec)}
