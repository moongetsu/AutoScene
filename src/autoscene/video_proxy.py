"""Browser-playable video proxy for Chromium <video>."""

from __future__ import annotations

import json
import logging
import subprocess
import threading
from dataclasses import dataclass, field
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

_jobs_guard = threading.Lock()
_jobs: dict[str, "_BuildJob"] = {}


class ProxyNotReady(RuntimeError):
    """Proxy missing; a background build is running or was kicked."""

    def __init__(self, *, building: bool, error: str | None = None) -> None:
        self.building = building
        self.error = error
        msg = error or ("Browser proxy still building" if building else "Browser proxy not ready")
        super().__init__(msg)


@dataclass
class _BuildJob:
    key: str
    source: Path
    cache_dir: Path
    state: str = "building"  # building | ready | error
    error: str | None = None
    thread: threading.Thread | None = field(default=None, repr=False)


def ensure_browser_playable(
    source: Path,
    cache_dir: Path,
    *,
    wait: bool = False,
) -> Path:
    """
    Return a path Chromium can play in <video>.

    By default (wait=False) kicks a background encode when needed and raises
    ProxyNotReady so HTTP handlers stay responsive. Pass wait=True only for
    offline/CLI paths that must block until the file exists.
    """
    playable = try_browser_playable(source, cache_dir)
    if playable is not None:
        return playable

    info = request_browser_proxy(source, cache_dir)
    if info.get("proxy_ready"):
        path = try_browser_playable(source, cache_dir)
        if path is not None:
            return path

    if not wait:
        raise ProxyNotReady(
            building=bool(info.get("building")),
            error=str(info["build_error"]) if info.get("build_error") else None,
        )

    key = _job_key(cache_dir)
    job = _jobs.get(key)
    if job and job.thread is not None:
        job.thread.join()
    playable = try_browser_playable(source, cache_dir)
    if playable is not None:
        return playable
    err = None
    with _jobs_guard:
        job = _jobs.get(key)
        if job is not None:
            err = job.error
    raise RuntimeError(err or "Could not build browser-playable proxy")


def try_browser_playable(source: Path, cache_dir: Path) -> Path | None:
    """Return playable path if already available; never starts an encode."""
    src = source.resolve()
    if not src.is_file():
        raise FileNotFoundError(str(src))

    if src.suffix.lower() in _BROWSER_OK_EXT and is_chromium_safe(src):
        return src

    out = cache_dir / _PROXY_NAME
    if _usable_proxy(out, src):
        return out
    return None


def request_browser_proxy(source: Path, cache_dir: Path) -> dict[str, object]:
    """
    Ensure a background proxy build is running when needed.
    Always returns quickly with status fields (never waits on ffmpeg).
    Never starts a second writer if one ffmpeg is already targeting the proxy.
    """
    src = source.resolve()
    if not src.is_file():
        raise FileNotFoundError(str(src))

    cache_dir = cache_dir.resolve()
    cache_dir.mkdir(parents=True, exist_ok=True)
    key = str(cache_dir)

    if src.suffix.lower() in _BROWSER_OK_EXT and is_chromium_safe(src):
        return {
            "proxy_ready": True,
            "building": False,
            "build_error": None,
            "native_ok": True,
        }

    out = cache_dir / _PROXY_NAME
    if _usable_proxy(out, src):
        with _jobs_guard:
            job = _jobs.get(key)
            if job is not None:
                job.state = "ready"
                job.error = None
        return {
            "proxy_ready": True,
            "building": False,
            "build_error": None,
            "native_ok": False,
        }

    # Collapse duplicate orphan encoders (API restarts used to stack them).
    _ensure_single_proxy_writer(out)

    with _jobs_guard:
        job = _jobs.get(key)
        if job is not None and job.state == "building" and job.thread and job.thread.is_alive():
            return {
                "proxy_ready": False,
                "building": True,
                "build_error": None,
                "native_ok": False,
            }
        if _proxy_writer_active(out):
            # External/orphan ffmpeg already owns this output — adopt it.
            return {
                "proxy_ready": False,
                "building": True,
                "build_error": None,
                "native_ok": False,
            }
        # Retry after error, or start fresh.
        job = _BuildJob(key=key, source=src, cache_dir=cache_dir, state="building")
        thread = threading.Thread(
            target=_run_build_job,
            args=(job,),
            daemon=True,
            name=f"browser-proxy-{cache_dir.name}",
        )
        job.thread = thread
        _jobs[key] = job
        thread.start()

    return {
        "proxy_ready": False,
        "building": True,
        "build_error": None,
        "native_ok": False,
    }


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

    building = False
    build_error: str | None = None
    with _jobs_guard:
        job = _jobs.get(str(cache_dir.resolve()))
        if job is not None:
            if job.state == "building" and job.thread and job.thread.is_alive():
                building = True
            elif job.state == "error":
                build_error = job.error
    if not ready and not building and _proxy_writer_active(out):
        building = True

    return {
        "source_suffix": suffix,
        "source_video_codec": vcodec,
        "source_audio_codec": acodec,
        "native_ok": native_ok,
        "proxy_ready": ready,
        "proxy_path": str(out) if out.exists() else None,
        "building": building,
        "build_error": build_error,
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


def _job_key(cache_dir: Path) -> str:
    return str(cache_dir.resolve())


def _run_build_job(job: _BuildJob) -> None:
    lock = _lock_for(job.key)
    try:
        with lock:
            if _usable_proxy(job.cache_dir / _PROXY_NAME, job.source):
                with _jobs_guard:
                    job.state = "ready"
                    job.error = None
                return
            _build_proxy(job.source, job.cache_dir)
        with _jobs_guard:
            if _usable_proxy(job.cache_dir / _PROXY_NAME, job.source):
                job.state = "ready"
                job.error = None
            else:
                job.state = "error"
                job.error = "Proxy build finished but file is not browser-safe"
    except Exception as exc:  # noqa: BLE001
        log.exception("browser proxy build failed key=%s", job.key)
        with _jobs_guard:
            job.state = "error"
            job.error = str(exc)


def _build_proxy(src: Path, cache_dir: Path) -> Path:
    """Encode/remux under caller-held lock. Raises on failure."""
    out = cache_dir / _PROXY_NAME
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
    # Prefer NVENC/VideoToolbox; always format=yuv420p so Main10 HEVC cannot
    # hang h264_nvenc (10-bit → 8-bit without an explicit convert).
    for cmd in _preview_encode_commands(ffmpeg, src, out):
        enc = _encoder_label(cmd)
        log.info("browser proxy encode attempt encoder=%s out=%s", enc, out)
        if _run_ffmpeg(cmd) and is_chromium_safe(out):
            return out

    raise RuntimeError(
        f"Could not build browser-playable H.264/AAC from {src.name} "
        f"(source codecs: video={vcodec}, audio={acodec}). "
        "Install ffmpeg with libx264 (and NVENC if available)."
    )


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


def _preview_encode_commands(ffmpeg: str, src: Path, out: Path) -> list[list[str]]:
    """Ordered encode attempts: GPU H.264 first, then libx264."""
    vf = "scale='min(1280,iw)':-2,format=yuv420p"
    maps = ["-map", "0:v:0", "-map", "0:a:0?"]
    audio_out = [
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
    attempts: list[list[str]] = []

    try:
        from autoscene.ffmpeg_encode import (
            hwaccel_input_args,
            nvenc_h264_available,
            videotoolbox_h264_available,
        )
    except Exception:  # noqa: BLE001
        hwaccel_input_args = lambda **_kw: []  # type: ignore[misc, assignment]
        nvenc_h264_available = lambda: False  # type: ignore[assignment]
        videotoolbox_h264_available = lambda: False  # type: ignore[assignment]

    # Software filters (scale + format) → no CUDA zero-copy; still use CUDA decode.
    hw = hwaccel_input_args(zero_copy=False)

    if nvenc_h264_available():
        attempts.append(
            [
                ffmpeg,
                "-y",
                *hw,
                "-i",
                str(src),
                *maps,
                "-vf",
                vf,
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-cq",
                "23",
                "-b:v",
                "0",
                "-pix_fmt",
                "yuv420p",
                *audio_out,
            ]
        )

    if videotoolbox_h264_available():
        attempts.append(
            [
                ffmpeg,
                "-y",
                *hw,
                "-i",
                str(src),
                *maps,
                "-vf",
                vf,
                "-c:v",
                "h264_videotoolbox",
                "-b:v",
                "4M",
                "-pix_fmt",
                "yuv420p",
                *audio_out,
            ]
        )

    attempts.append(
        [
            ffmpeg,
            "-y",
            "-i",
            str(src),
            *maps,
            "-vf",
            vf,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            *audio_out,
        ]
    )
    return attempts


def _encoder_label(cmd: list[str]) -> str:
    try:
        i = cmd.index("-c:v")
        return cmd[i + 1]
    except (ValueError, IndexError):
        return "unknown"


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


def _proxy_writer_active(out: Path) -> bool:
    """True when an ffmpeg process is currently writing this proxy path."""
    return bool(_ffmpeg_pids_writing(out))


def _ensure_single_proxy_writer(out: Path) -> None:
    """If several ffmpeg jobs target the same proxy, keep one and stop the rest."""
    pids = _ffmpeg_pids_writing(out)
    if len(pids) <= 1:
        return
    # Keep the newest PID (usually the intentional restart); kill older orphans.
    keep = max(pids)
    for pid in pids:
        if pid == keep:
            continue
        try:
            import os

            os.kill(pid, 9)
            log.warning("killed duplicate browser_proxy ffmpeg pid=%s keep=%s", pid, keep)
        except OSError as exc:
            log.warning("could not kill duplicate ffmpeg pid=%s: %s", pid, exc)


def _ffmpeg_pids_writing(out: Path) -> list[int]:
    needle = str(out.resolve()).lower().replace("/", "\\")
    name = out.name.lower()
    found: list[int] = []
    try:
        # Windows: PowerShell CIM is available on the user's box.
        import sys

        if sys.platform.startswith("win"):
            ps = (
                "Get-CimInstance Win32_Process -Filter \"Name='ffmpeg.exe'\" | "
                "Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"
            )
            proc = subprocess.run(
                ["powershell", "-NoProfile", "-Command", ps],
                check=False,
                capture_output=True,
                text=True,
                timeout=8,
            )
            raw = (proc.stdout or "").strip()
            if not raw:
                return []
            import json

            data = json.loads(raw)
            rows = data if isinstance(data, list) else [data]
            for row in rows:
                cmd = str(row.get("CommandLine") or "").lower().replace("/", "\\")
                pid = int(row.get("ProcessId") or 0)
                if pid and name in cmd and needle in cmd:
                    found.append(pid)
            return found
    except Exception as exc:  # noqa: BLE001
        log.debug("ffmpeg process scan failed: %s", exc)
    return found
