"""Browser-playable video proxy for Chromium <video>."""

from __future__ import annotations

import json
import logging
import subprocess
import threading
from pathlib import Path

from autoscene.export import ensure_ffmpeg

log = logging.getLogger(__name__)

_BROWSER_OK_EXT = frozenset({".mp4", ".webm", ".m4v"})
# ffprobe codec_name values Chromium generally accepts in <video>
_SAFE_VIDEO = frozenset({"h264", "vp8", "vp9", "av1"})
_SAFE_AUDIO = frozenset({"aac", "mp3", "opus", "vorbis", "flac"})

_PROXY_NAME = "browser_proxy.mp4"
_locks: dict[str, threading.Lock] = {}
_locks_guard = threading.Lock()


def ensure_browser_playable(source: Path, cache_dir: Path) -> Path:
    """
    Return a path Chromium can play in <video>.
    Stream-copy into MP4 only when codecs are already browser-safe.
    Otherwise build a downscaled H.264/AAC preview (HEVC/E-AC3 etc. fail in Chromium).
    """
    src = source.resolve()
    if not src.is_file():
        raise FileNotFoundError(str(src))

    if src.suffix.lower() in _BROWSER_OK_EXT and is_chromium_safe(src):
        return src

    cache_dir.mkdir(parents=True, exist_ok=True)
    out = cache_dir / _PROXY_NAME
    lock = _lock_for(str(cache_dir.resolve()))
    with lock:
        if _usable_proxy(out, src):
            return out

        if out.exists():
            try:
                out.unlink()
            except OSError:
                pass

        ffmpeg = ensure_ffmpeg()
        vcodec, acodec = probe_codecs(src)
        log.info(
            "Building browser video proxy src=%s codecs=%s/%s out=%s",
            src,
            vcodec,
            acodec,
            out,
        )

        # Fast path: remux only when already Chromium-safe (rare for .mkv anime).
        if vcodec in _SAFE_VIDEO and (acodec is None or acodec in _SAFE_AUDIO):
            if _run_ffmpeg(
                [
                    ffmpeg,
                    "-y",
                    "-i",
                    str(src),
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0?",
                    "-c",
                    "copy",
                    "-movflags",
                    "+faststart",
                    str(out),
                ]
            ) and is_chromium_safe(out):
                return out

        # Preview re-encode: H.264 8-bit + AAC, max width 1280 (reel/UI, not archive).
        if _run_ffmpeg(
            [
                ffmpeg,
                "-y",
                "-i",
                str(src),
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-vf",
                "scale='min(1280,iw)':-2",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ac",
                "2",
                "-movflags",
                "+faststart",
                str(out),
            ]
        ) and is_chromium_safe(out):
            return out

        raise RuntimeError(
            f"Could not build browser-playable H.264/AAC from {src.name} "
            f"(source codecs: video={vcodec}, audio={acodec}). "
            "Install ffmpeg with libx264."
        )


def proxy_status(source: Path, cache_dir: Path) -> dict[str, object]:
    src = source.resolve()
    suffix = src.suffix.lower() if src.exists() else ""
    native_ok = suffix in _BROWSER_OK_EXT and src.exists() and is_chromium_safe(src)
    out = cache_dir / _PROXY_NAME
    ready = native_ok or _usable_proxy(out, src)
    vcodec, acodec = (None, None)
    try:
        if src.exists():
            vcodec, acodec = probe_codecs(src)
    except Exception:  # noqa: BLE001
        pass
    return {
        "source_suffix": suffix,
        "source_video_codec": vcodec,
        "source_audio_codec": acodec,
        "native_ok": native_ok,
        "proxy_ready": ready,
        "proxy_path": str(out) if out.exists() else None,
        "needs_transcode": not ready
        and (
            vcodec not in _SAFE_VIDEO
            or (acodec is not None and acodec not in _SAFE_AUDIO)
            or suffix not in _BROWSER_OK_EXT
        ),
    }


def is_chromium_safe(path: Path) -> bool:
    try:
        vcodec, acodec = probe_codecs(path)
    except Exception:  # noqa: BLE001
        return False
    if vcodec not in _SAFE_VIDEO:
        return False
    if acodec is not None and acodec not in _SAFE_AUDIO:
        return False
    return True


def probe_codecs(path: Path) -> tuple[str | None, str | None]:
    ffprobe = _ensure_ffprobe()
    proc = subprocess.run(
        [
            ffprobe,
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name",
            "-of",
            "json",
            str(path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or "ffprobe failed")[-300:])
    data = json.loads(proc.stdout or "{}")
    vcodec = None
    acodec = None
    for stream in data.get("streams") or []:
        kind = stream.get("codec_type")
        name = stream.get("codec_name")
        if kind == "video" and vcodec is None:
            vcodec = name
        elif kind == "audio" and acodec is None:
            acodec = name
    return vcodec, acodec


def _usable_proxy(out: Path, src: Path) -> bool:
    try:
        if not out.is_file() or out.stat().st_size <= 1024:
            return False
        if src.exists() and out.stat().st_mtime < src.stat().st_mtime:
            return False
    except OSError:
        return False
    return is_chromium_safe(out)


def _lock_for(key: str) -> threading.Lock:
    with _locks_guard:
        lock = _locks.get(key)
        if lock is None:
            lock = threading.Lock()
            _locks[key] = lock
        return lock


def _ensure_ffprobe() -> str:
    import shutil

    exe = shutil.which("ffprobe")
    if not exe:
        # Same package as ffmpeg on WinGet layout
        ffmpeg = ensure_ffmpeg()
        sibling = Path(ffmpeg).with_name("ffprobe.exe" if ffmpeg.lower().endswith(".exe") else "ffprobe")
        if sibling.is_file():
            return str(sibling)
        raise RuntimeError("ffprobe not found on PATH")
    return exe


def _run_ffmpeg(cmd: list[str]) -> bool:
    try:
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        log.warning("ffmpeg launch failed: %s", exc)
        return False
    if proc.returncode == 0:
        return True
    tail = (proc.stderr or "")[-500:].strip()
    log.warning("ffmpeg proxy step failed code=%s: %s", proc.returncode, tail)
    try:
        out = Path(cmd[-1])
        if out.is_file():
            out.unlink(missing_ok=True)
    except OSError:
        pass
    return False
