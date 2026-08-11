"""Short per-scene H.264 clips for Export hover preview.

Independent of the full-film browser_proxy encode so hover works in seconds
even while the long HEVC→H.264 proxy is still building.
"""

from __future__ import annotations

import logging
import subprocess
import threading
from pathlib import Path

from autoscene.export import ensure_ffmpeg

log = logging.getLogger(__name__)

_CLIP_DIR = "hover_clips_v1"
_locks: dict[str, threading.Lock] = {}
_locks_guard = threading.Lock()
_jobs_guard = threading.Lock()
_jobs: dict[str, "_ClipJob"] = {}
_build_sem = threading.Semaphore(3)


class _ClipJob:
    def __init__(self, key: str) -> None:
        self.key = key
        self.state = "building"  # building | ready | error
        self.error: str | None = None
        self.thread: threading.Thread | None = None


def hover_clip_path(cache_dir: Path, start_sec: float, end_sec: float) -> Path:
    a = max(0, int(round(float(start_sec) * 1000)))
    b = max(a + 50, int(round(float(end_sec) * 1000)))
    return cache_dir / _CLIP_DIR / f"{a:08d}_{b:08d}.mp4"


def try_hover_clip(
    cache_dir: Path, start_sec: float, end_sec: float
) -> Path | None:
    out = hover_clip_path(cache_dir, start_sec, end_sec)
    if out.is_file() and out.stat().st_size > 1024 and _looks_playable(out):
        return out
    return None


def request_hover_clip(
    source: Path,
    cache_dir: Path,
    start_sec: float,
    end_sec: float,
) -> dict[str, object]:
    """Return quickly; builds in a background thread when missing."""
    src = source.resolve()
    if not src.is_file():
        raise FileNotFoundError(str(src))

    start, end = _normalize_range(start_sec, end_sec)
    out = hover_clip_path(cache_dir, start, end)
    out.parent.mkdir(parents=True, exist_ok=True)

    if try_hover_clip(cache_dir, start, end) is not None:
        return {"ready": True, "building": False, "path": str(out), "error": None}

    key = str(out.resolve())
    with _jobs_guard:
        job = _jobs.get(key)
        if job is not None and job.state == "building" and job.thread and job.thread.is_alive():
            return {"ready": False, "building": True, "path": str(out), "error": None}
        job = _ClipJob(key)
        thread = threading.Thread(
            target=_run_clip_job,
            args=(job, src, cache_dir, start, end, out),
            daemon=True,
            name=f"hover-clip-{out.stem}",
        )
        job.thread = thread
        _jobs[key] = job
        thread.start()

    return {"ready": False, "building": True, "path": str(out), "error": None}


def hover_clip_status(
    cache_dir: Path, start_sec: float, end_sec: float
) -> dict[str, object]:
    start, end = _normalize_range(start_sec, end_sec)
    out = hover_clip_path(cache_dir, start, end)
    ready = try_hover_clip(cache_dir, start, end) is not None
    building = False
    error: str | None = None
    with _jobs_guard:
        job = _jobs.get(str(out.resolve()))
        if job is not None:
            if job.state == "building" and job.thread and job.thread.is_alive():
                building = True
            elif job.state == "error":
                error = job.error
    return {
        "ready": ready,
        "building": building,
        "error": error,
        "path": str(out) if out.exists() else None,
    }


def prefetch_hover_clips(
    source: Path,
    cache_dir: Path,
    ranges: list[tuple[float, float]],
) -> dict[str, object]:
    """Kick background builds for many scene windows (deduped)."""
    started = 0
    ready = 0
    seen: set[str] = set()
    for a, b in ranges:
        start, end = _normalize_range(a, b)
        out = hover_clip_path(cache_dir, start, end)
        key = str(out.resolve())
        if key in seen:
            continue
        seen.add(key)
        if try_hover_clip(cache_dir, start, end) is not None:
            ready += 1
            continue
        request_hover_clip(source, cache_dir, start, end)
        started += 1
    return {"total": len(seen), "ready": ready, "started": started}


def _run_clip_job(
    job: _ClipJob,
    src: Path,
    cache_dir: Path,
    start: float,
    end: float,
    out: Path,
) -> None:
    lock = _lock_for(str(out.resolve()))
    try:
        with lock:
            if try_hover_clip(cache_dir, start, end) is not None:
                with _jobs_guard:
                    job.state = "ready"
                return
            with _build_sem:
                _build_hover_clip(src, cache_dir, start, end, out)
        with _jobs_guard:
            if try_hover_clip(cache_dir, start, end) is not None:
                job.state = "ready"
                job.error = None
            else:
                job.state = "error"
                job.error = "Hover clip build finished but file is not playable"
    except Exception as exc:  # noqa: BLE001
        log.exception("hover clip failed out=%s", out)
        with _jobs_guard:
            job.state = "error"
            job.error = str(exc)


def _build_hover_clip(
    src: Path,
    cache_dir: Path,
    start: float,
    end: float,
    out: Path,
) -> None:
    if out.exists():
        try:
            out.unlink()
        except OSError:
            pass

    ffmpeg = ensure_ffmpeg()
    dur = max(0.12, end - start)
    # Cap extreme dialogue plates so hover stays snappy.
    dur = min(dur, 45.0)

    candidates: list[Path] = []
    proxy = cache_dir / "browser_proxy.mp4"
    # Only use proxy when fully playable (incomplete faststart files break seeks).
    try:
        from autoscene.video_proxy import _usable_proxy

        if _usable_proxy(proxy, src):
            candidates.append(proxy)
    except Exception:  # noqa: BLE001
        if proxy.is_file() and proxy.stat().st_size > 1024 * 1024:
            # Heuristic: skip incomplete (no moov) — probe below in attempts.
            pass
    if src.is_file():
        candidates.append(src)
    if not candidates:
        raise FileNotFoundError(str(src))

    hw = _hw_args()
    for src_try in candidates:
        for cmd in _encode_commands(ffmpeg, src_try, start, dur, out, hw):
            log.info(
                "hover clip encode src=%s start=%.3f dur=%.3f out=%s",
                src_try.name,
                start,
                dur,
                out.name,
            )
            if _run_ffmpeg(cmd) and _looks_playable(out):
                return
            try:
                out.unlink(missing_ok=True)
            except OSError:
                pass

    raise RuntimeError(
        f"Could not build hover clip {start:.2f}-{end:.2f}s from {src.name}"
    )


def _encode_commands(
    ffmpeg: str,
    src: Path,
    start: float,
    dur: float,
    out: Path,
    hw: list[str],
) -> list[list[str]]:
    vf = "scale='min(720,iw)':-2,format=yuv420p"
    head = [
        ffmpeg,
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
    ]
    # Input seek — muted hover loops (Export tiles are silent).
    inputs = ["-ss", f"{start:.3f}", "-i", str(src), "-t", f"{dur:.3f}", "-map", "0:v:0"]
    tail = ["-vf", vf, "-an", "-movflags", "+faststart", str(out)]
    cmds: list[list[str]] = []

    try:
        from autoscene.ffmpeg_encode import nvenc_h264_available

        if nvenc_h264_available():
            cmds.append(
                [
                    *head,
                    *hw,
                    *inputs,
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p4",
                    "-rc",
                    "vbr",
                    "-cq",
                    "28",
                    "-b:v",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                    *tail,
                ]
            )
    except Exception:  # noqa: BLE001
        pass

    cmds.append(
        [
            *head,
            *inputs,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "26",
            "-pix_fmt",
            "yuv420p",
            *tail,
        ]
    )
    return cmds


def _hw_args() -> list[str]:
    try:
        from autoscene.ffmpeg_encode import hwaccel_input_args

        return list(hwaccel_input_args(zero_copy=False))
    except Exception:  # noqa: BLE001
        return []


def _normalize_range(start_sec: float, end_sec: float) -> tuple[float, float]:
    a = max(0.0, float(start_sec))
    b = max(a + 0.12, float(end_sec))
    return a, b


def _lock_for(key: str) -> threading.Lock:
    with _locks_guard:
        lock = _locks.get(key)
        if lock is None:
            lock = threading.Lock()
            _locks[key] = lock
        return lock


def _looks_playable(path: Path) -> bool:
    if not path.is_file() or path.stat().st_size <= 1024:
        return False
    try:
        from autoscene.video_proxy import is_chromium_safe

        return is_chromium_safe(path)
    except Exception:  # noqa: BLE001
        # ffprobe unavailable — size heuristic only.
        return path.stat().st_size > 8 * 1024


def _run_ffmpeg(cmd: list[str]) -> bool:
    try:
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            timeout=180,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        log.warning("hover clip ffmpeg failed: %s", exc)
        return False
    if proc.returncode == 0:
        return True
    tail = (proc.stderr or "")[-400:].strip()
    log.warning("hover clip ffmpeg code=%s: %s", proc.returncode, tail)
    return False
