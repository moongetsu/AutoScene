"""FFmpeg encode helpers for AutoScene export.

Modes:
  quality  — GPU NVENC delivery encode (balanced CQ). Default when re-encode needed.
  lossless — archive / master: visually transparent NVENC matched to source bitrate
             (not giant bit-exact FFV1/x264). Keeps original look without ballooning size.
  copy     — stream copy (H.264-safe sources only).
"""

from __future__ import annotations

import json
import logging
import shutil
import subprocess
import sys
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Literal

log = logging.getLogger(__name__)

ExportMode = Literal["copy", "quality", "lossless"]
VideoCodecPref = Literal[
    "h264_nvenc",
    "hevc_nvenc",
    "av1_nvenc",
    "h264_videotoolbox",
    "hevc_videotoolbox",
    "auto",
]

_NVENC_CODECS = frozenset({"h264_nvenc", "hevc_nvenc", "av1_nvenc"})
_VIDEOTOOLBOX_CODECS = frozenset({"h264_videotoolbox", "hevc_videotoolbox"})

# Everything the API will accept for video_codec. Kept here so the whitelist
# cannot drift from VideoCodecPref.
VIDEO_CODEC_CHOICES = frozenset(_NVENC_CODECS | _VIDEOTOOLBOX_CODECS | {"auto"})


def is_gpu_encoder(enc: str) -> bool:
    """NVENC and VideoToolbox both count as hardware encode."""
    return enc in _NVENC_CODECS or enc in _VIDEOTOOLBOX_CODECS


_SAFE_AUDIO_COPY = frozenset(
    {"aac", "mp3", "opus", "vorbis", "flac", "pcm_s16le", "pcm_s24le"}
)


def ensure_ffmpeg() -> str:
    """
    Resolve ffmpeg binary.

    Pin channel (first hit wins):
      1) AUTOSCENE_FFMPEG — absolute path to ffmpeg(.exe)
      2) data_dir/tools/ffmpeg/ffmpeg(.exe) — optional bundled build
      3) PATH — system ffmpeg

    If AUTOSCENE_FFMPEG_SHA256 is set, the chosen binary must match.
    Requires a usable build with libx264 (preview/proxy path).
    """
    import os

    from autoscene.config import settings

    candidates: list[Path] = []
    env = (os.environ.get("AUTOSCENE_FFMPEG") or "").strip()
    if env:
        candidates.append(Path(env))
    settings.ensure_dirs()
    tools = settings.data_dir / "tools" / "ffmpeg"
    candidates.append(tools / ("ffmpeg.exe" if os.name == "nt" else "ffmpeg"))
    which = shutil.which("ffmpeg")
    if which:
        candidates.append(Path(which))

    exe: Path | None = None
    for c in candidates:
        if c.is_file():
            exe = c.resolve()
            break
    if exe is None:
        raise RuntimeError(
            "ffmpeg not found. Install ffmpeg on PATH, or set AUTOSCENE_FFMPEG, "
            "or place a build at data_dir/tools/ffmpeg/."
        )

    expect_sha = (os.environ.get("AUTOSCENE_FFMPEG_SHA256") or "").strip().lower()
    if expect_sha:
        import hashlib

        h = hashlib.sha256()
        with exe.open("rb") as f:
            while True:
                chunk = f.read(1024 * 1024)
                if not chunk:
                    break
                h.update(chunk)
        got = h.hexdigest()
        if got != expect_sha:
            raise RuntimeError(
                f"ffmpeg SHA256 mismatch for {exe}: got {got}, expected {expect_sha}"
            )

    try:
        ver = subprocess.run(
            [str(exe), "-hide_banner", "-version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=8,
        )
    except OSError as exc:
        raise RuntimeError(f"Cannot run ffmpeg at {exe}") from exc
    banner = ((ver.stdout or "") + (ver.stderr or "")).strip()
    head = banner.splitlines()[0] if banner else str(exe)
    log.info("ffmpeg pin: %s (%s)", exe, head)

    try:
        enc = subprocess.run(
            [str(exe), "-hide_banner", "-encoders"],
            check=False,
            capture_output=True,
            text=True,
            timeout=12,
        )
    except OSError as exc:
        raise RuntimeError(f"Cannot probe ffmpeg encoders at {exe}") from exc
    enc_text = (enc.stdout or "") + (enc.stderr or "")
    if "libx264" not in enc_text:
        raise RuntimeError(
            f"ffmpeg at {exe} is missing libx264 (needed for preview/proxy). "
            "Install a full build (e.g. gyan.dev / BtbN essentials+)."
        )
    return str(exe)


def _ffmpeg() -> str:
    return ensure_ffmpeg()


@dataclass(frozen=True)
class SourceEncodeProfile:
    width: int
    height: int
    video_bitrate: int | None
    audio_bitrate: int | None
    fps: float | None
    pix_fmt: str | None
    vcodec: str | None
    acodec: str | None
    bits_per_raw_sample: int | None
    color_range: str | None
    color_space: str | None
    color_transfer: str | None
    color_primaries: str | None


@lru_cache(maxsize=1)
def _encoder_list() -> str:
    ffmpeg = _ffmpeg()
    try:
        proc = subprocess.run(
            [ffmpeg, "-hide_banner", "-encoders"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return ""
    return (proc.stdout or "") + (proc.stderr or "")


@lru_cache(maxsize=1)
def _hwaccel_list() -> str:
    ffmpeg = _ffmpeg()
    try:
        proc = subprocess.run(
            [ffmpeg, "-hide_banner", "-hwaccels"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return ""
    return (proc.stdout or "") + (proc.stderr or "")


def cuda_hwaccel_available() -> bool:
    return "cuda" in _hwaccel_list().lower()


def videotoolbox_hwaccel_available() -> bool:
    return "videotoolbox" in _hwaccel_list().lower()


def nvenc_h264_available() -> bool:
    return "h264_nvenc" in _encoder_list()


def nvenc_hevc_available() -> bool:
    return "hevc_nvenc" in _encoder_list()


def nvenc_av1_available() -> bool:
    return "av1_nvenc" in _encoder_list()


def videotoolbox_h264_available() -> bool:
    return "h264_videotoolbox" in _encoder_list()


def videotoolbox_hevc_available() -> bool:
    return "hevc_videotoolbox" in _encoder_list()


def ffv1_available() -> bool:
    return "ffv1" in _encoder_list()


def encoder_available(enc: str) -> bool:
    return {
        "h264_nvenc": nvenc_h264_available,
        "hevc_nvenc": nvenc_hevc_available,
        "av1_nvenc": nvenc_av1_available,
        "h264_videotoolbox": videotoolbox_h264_available,
        "hevc_videotoolbox": videotoolbox_hevc_available,
    }.get(enc, lambda: False)()


def available_export_codecs() -> dict[str, bool]:
    return {
        "h264_nvenc": nvenc_h264_available(),
        "hevc_nvenc": nvenc_hevc_available(),
        "av1_nvenc": nvenc_av1_available(),
        "h264_videotoolbox": videotoolbox_h264_available(),
        "hevc_videotoolbox": videotoolbox_hevc_available(),
        "cuda_hwaccel": cuda_hwaccel_available(),
        "videotoolbox_hwaccel": videotoolbox_hwaccel_available(),
    }


def probe_encode_profile(video_path: Path) -> SourceEncodeProfile:
    from autoscene.video_proxy import _ensure_ffprobe

    ffprobe = _ensure_ffprobe()
    proc = subprocess.run(
        [
            ffprobe,
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            str(video_path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or "ffprobe failed")[-300:])
    data = json.loads(proc.stdout or "{}")

    width = 0
    height = 0
    v_bitrate: int | None = None
    a_bitrate: int | None = None
    fps: float | None = None
    pix_fmt: str | None = None
    vcodec: str | None = None
    acodec: str | None = None
    bits: int | None = None
    color_range: str | None = None
    color_space: str | None = None
    color_transfer: str | None = None
    color_primaries: str | None = None

    for stream in data.get("streams") or []:
        kind = stream.get("codec_type")
        if kind == "video" and vcodec is None:
            vcodec = stream.get("codec_name")
            width = int(stream.get("width") or 0)
            height = int(stream.get("height") or 0)
            pix_fmt = stream.get("pix_fmt")
            bits = _parse_int(stream.get("bits_per_raw_sample"))
            color_range = stream.get("color_range")
            color_space = stream.get("color_space")
            color_transfer = stream.get("color_transfer")
            color_primaries = stream.get("color_primaries")
            br = _parse_int(stream.get("bit_rate"))
            if br:
                v_bitrate = br
            fps = _parse_fps(stream.get("avg_frame_rate")) or _parse_fps(
                stream.get("r_frame_rate")
            )
        elif kind == "audio" and acodec is None:
            acodec = stream.get("codec_name")
            a_bitrate = _parse_int(stream.get("bit_rate"))

    fmt = data.get("format") or {}
    fmt_br = _parse_int(fmt.get("bit_rate"))
    if v_bitrate is None and fmt_br:
        v_bitrate = max(0, fmt_br - (a_bitrate or 0)) or fmt_br

    return SourceEncodeProfile(
        width=width,
        height=height,
        video_bitrate=v_bitrate,
        audio_bitrate=a_bitrate,
        fps=fps,
        pix_fmt=pix_fmt,
        vcodec=vcodec,
        acodec=acodec,
        bits_per_raw_sample=bits,
        color_range=color_range if color_range not in (None, "unknown") else None,
        color_space=color_space if color_space not in (None, "unknown") else None,
        color_transfer=color_transfer if color_transfer not in (None, "unknown") else None,
        color_primaries=color_primaries
        if color_primaries not in (None, "unknown")
        else None,
    )


def bit_depth(profile: SourceEncodeProfile) -> int:
    if profile.bits_per_raw_sample and profile.bits_per_raw_sample >= 8:
        return int(profile.bits_per_raw_sample)
    pix = profile.pix_fmt or ""
    if "12" in pix:
        return 12
    if "10" in pix:
        return 10
    if "9" in pix:
        return 9
    return 8


def preserve_pix_fmt(profile: SourceEncodeProfile) -> str:
    if profile.pix_fmt:
        return profile.pix_fmt
    return "yuv420p10le" if bit_depth(profile) >= 10 else "yuv420p"


def container_of(path: Path | str) -> str:
    return Path(path).suffix.lower().lstrip(".") or "mp4"


def part_suffix_for_container(container: str) -> str:
    c = container.lower().lstrip(".")
    if c == "mkv":
        return ".mkv"
    if c == "mov":
        return ".mov"
    return ".mp4"


def video_filter_args(profile: SourceEncodeProfile) -> list[str]:
    """Only force even size when needed; never downscale."""
    if profile.width > 0 and profile.height > 0:
        if profile.width % 2 == 0 and profile.height % 2 == 0:
            return []
    return ["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]


def color_tag_args(profile: SourceEncodeProfile) -> list[str]:
    args: list[str] = []
    if profile.color_range:
        args.extend(["-color_range", profile.color_range])
    if profile.color_space:
        args.extend(["-colorspace", profile.color_space])
    if profile.color_transfer:
        args.extend(["-color_trc", profile.color_transfer])
    if profile.color_primaries:
        args.extend(["-color_primaries", profile.color_primaries])
    return args


def hwaccel_input_args(*, zero_copy: bool = False) -> list[str]:
    """
    Hardware decode when available: CUDA, else VideoToolbox on macOS.

    zero_copy=True keeps frames on GPU for NVENC (no software filters allowed).
    VideoToolbox has no equivalent zero-copy handoff to its encoder, so the flag
    is ignored there and frames round-trip through system memory.
    """
    if cuda_hwaccel_available():
        if zero_copy:
            return ["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]
        return ["-hwaccel", "cuda"]
    if videotoolbox_hwaccel_available():
        return ["-hwaccel", "videotoolbox"]
    return []


def prefer_output_path(
    output_path: Path,
    *,
    mode: ExportMode,
    codec: VideoCodecPref | None = None,
) -> Path:
    """Fill missing/invalid suffix. Respect explicit .mp4/.mov/.mkv."""
    codec = codec or default_video_codec()
    if output_path.suffix.lower() in {".mp4", ".mov", ".mkv"}:
        return output_path
    # VideoToolbox HEVC stays in mp4 — hvc1 tagging keeps it QuickTime-playable,
    # and mkv is a worse default on macOS.
    if mode in {"quality", "lossless"} and codec in {"hevc_nvenc", "av1_nvenc"}:
        return output_path.with_suffix(".mkv")
    return output_path.with_suffix(".mp4")


# Back-compat alias
def prefer_lossless_output_path(output_path: Path, *, reencode: bool) -> Path:
    return prefer_output_path(
        output_path,
        mode="lossless" if reencode else "copy",
        codec=default_video_codec(),
    )


def _any_nvenc_available() -> bool:
    return nvenc_h264_available() or nvenc_hevc_available() or nvenc_av1_available()


def _any_videotoolbox_available() -> bool:
    return videotoolbox_h264_available() or videotoolbox_hevc_available()


def any_gpu_encoder_available() -> bool:
    return _any_nvenc_available() or _any_videotoolbox_available()


def _videotoolbox_args(
    encoder: str,
    *,
    pix: str,
    color: list[str],
    container: str = "mp4",
    quality: int | None = None,
    bitrate: int | None = None,
    maxrate: int | None = None,
) -> list[str]:
    """
    VideoToolbox rate control.

    Shares nothing with NVENC: no -cq/-preset/-tune/-rc/-rc-lookahead exist on
    these encoders. Constant quality is -q:v on a 0-100 scale (Apple Silicon
    only); otherwise target a bitrate. allow_sw lets the encode complete on
    machines where the media engine is busy or absent rather than hard-failing.
    """
    args = [
        "-c:v",
        encoder,
        "-allow_sw",
        "1",
        "-pix_fmt",
        pix,
        *color,
    ]
    # HEVC in mp4/mov defaults to the hev1 tag, which QuickTime and Finder
    # preview refuse to play. hvc1 is the same bitstream with a playable tag.
    if encoder == "hevc_videotoolbox" and container.lower().lstrip(".") in {
        "mp4",
        "mov",
    }:
        args.extend(["-tag:v", "hvc1"])
    if bitrate and bitrate > 0:
        mr = int(maxrate or bitrate)
        args.extend(["-b:v", str(bitrate), "-maxrate", str(mr), "-bufsize", str(mr * 2)])
    elif quality is not None:
        args.extend(["-q:v", str(quality)])
    return args


def _estimate_source_video_bitrate(profile: SourceEncodeProfile) -> int | None:
    """Bits/sec for archive bitrate match. Prefer probe; else resolution heuristic."""
    if profile.video_bitrate and profile.video_bitrate >= 400_000:
        return int(profile.video_bitrate)
    w, h = int(profile.width or 0), int(profile.height or 0)
    if w <= 0 or h <= 0:
        return None
    fps = float(profile.fps or 24.0)
    # ~0.07–0.10 bpp web/master heuristic; clamp to sane delivery range.
    est = int(w * h * max(12.0, fps) * 0.08)
    return max(4_000_000, min(est, 80_000_000))


def _nvenc_vbr_args(
    encoder: str,
    *,
    cq: int,
    pix: str,
    color: list[str],
    preset: str = "p5",
    bitrate: int | None = None,
    maxrate: int | None = None,
) -> list[str]:
    args = [
        "-c:v",
        encoder,
        "-preset",
        preset,
        "-tune",
        "hq",
        "-rc",
        "vbr",
        "-cq",
        str(cq),
        "-spatial_aq",
        "1",
        "-temporal_aq",
        "1",
        "-rc-lookahead",
        "32",
        "-pix_fmt",
        pix,
        *color,
    ]
    if bitrate and bitrate > 0:
        mr = int(maxrate or bitrate)
        args.extend(
            [
                "-b:v",
                str(bitrate),
                "-maxrate",
                str(mr),
                "-bufsize",
                str(mr * 2),
            ]
        )
    else:
        args.extend(["-b:v", "0"])
    return args


def _nvenc_quality_args(
    encoder: str,
    *,
    cq: int,
    pix: str,
    color: list[str],
) -> list[str]:
    return _nvenc_vbr_args(
        encoder, cq=cq, pix=pix, color=color, preset="p5", bitrate=None
    )


def default_video_codec() -> VideoCodecPref:
    """Platform's preferred hardware encoder (no ffmpeg probe — safe at import)."""
    return "h264_videotoolbox" if sys.platform == "darwin" else "h264_nvenc"


# A settings.json written on Windows can carry an NVENC pref onto a Mac.
_NVENC_TO_VIDEOTOOLBOX = {
    "h264_nvenc": "h264_videotoolbox",
    "hevc_nvenc": "hevc_videotoolbox",
    "av1_nvenc": "hevc_videotoolbox",  # no AV1 encode on VideoToolbox
}


def _codec_order(pref: VideoCodecPref, profile: SourceEncodeProfile) -> list[str]:
    is_hevc_src = (profile.vcodec or "").startswith("hevc")

    if sys.platform == "darwin":
        auto = "hevc_videotoolbox" if is_hevc_src else "h264_videotoolbox"
        key = auto if pref == "auto" else pref
        key = _NVENC_TO_VIDEOTOOLBOX.get(key, key)
        return {
            "h264_videotoolbox": ["h264_videotoolbox", "hevc_videotoolbox"],
            "hevc_videotoolbox": ["hevc_videotoolbox", "h264_videotoolbox"],
        }.get(key, ["h264_videotoolbox", "hevc_videotoolbox"])

    auto = "hevc_nvenc" if is_hevc_src else "h264_nvenc"
    key = auto if pref == "auto" else pref
    return {
        "h264_nvenc": ["h264_nvenc", "hevc_nvenc", "av1_nvenc"],
        "hevc_nvenc": ["hevc_nvenc", "h264_nvenc", "av1_nvenc"],
        "av1_nvenc": ["av1_nvenc", "hevc_nvenc", "h264_nvenc"],
    }.get(key, ["h264_nvenc", "hevc_nvenc", "av1_nvenc"])


def quality_video_encode_candidates(
    profile: SourceEncodeProfile,
    *,
    container: str,
    codec: VideoCodecPref = "h264_nvenc",
) -> list[tuple[list[str], str]]:
    """GPU NVENC delivery encodes. No CPU when NVENC exists."""
    c = container.lower().lstrip(".")
    depth = bit_depth(profile)
    color = color_tag_args(profile)
    candidates: list[tuple[list[str], str]] = []

    for enc in _codec_order(codec, profile):
        if enc == "h264_nvenc" and nvenc_h264_available():
            candidates.append(
                (
                    _nvenc_quality_args(
                        "h264_nvenc", cq=17, pix="yuv420p", color=color
                    ),
                    "h264_nvenc-quality",
                )
            )
        elif enc == "hevc_nvenc" and nvenc_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _nvenc_quality_args(
                            "hevc_nvenc", cq=18, pix="p010le", color=color
                        ),
                        "hevc_nvenc-quality-10",
                    )
                )
            candidates.append(
                (
                    _nvenc_quality_args(
                        "hevc_nvenc", cq=19, pix="yuv420p", color=color
                    ),
                    "hevc_nvenc-quality",
                )
            )
        elif enc == "av1_nvenc" and nvenc_av1_available():
            candidates.append(
                (
                    _nvenc_quality_args(
                        "av1_nvenc", cq=20, pix="yuv420p", color=color
                    ),
                    "av1_nvenc-quality",
                )
            )
        elif enc == "h264_videotoolbox" and videotoolbox_h264_available():
            # h264_videotoolbox takes nv12/yuv420p only — no 10-bit path.
            candidates.append(
                (
                    _videotoolbox_args(
                        "h264_videotoolbox", pix="yuv420p", color=color,
                        container=c, quality=65
                    ),
                    "h264_videotoolbox-quality",
                )
            )
            # -q:v constant quality is Apple Silicon only; bitrate always works.
            candidates.append(
                (
                    _videotoolbox_args(
                        "h264_videotoolbox",
                        pix="yuv420p",
                        color=color,
                        container=c,
                        bitrate=_estimate_source_video_bitrate(profile),
                    ),
                    "h264_videotoolbox-quality-vbr",
                )
            )
        elif enc == "hevc_videotoolbox" and videotoolbox_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _videotoolbox_args(
                            "hevc_videotoolbox",
                            pix="p010le",
                            color=color,
                            container=c,
                            quality=60,
                        ),
                        "hevc_videotoolbox-quality-10",
                    )
                )
            candidates.append(
                (
                    _videotoolbox_args(
                        "hevc_videotoolbox", pix="yuv420p", color=color,
                        container=c, quality=60
                    ),
                    "hevc_videotoolbox-quality",
                )
            )
            candidates.append(
                (
                    _videotoolbox_args(
                        "hevc_videotoolbox",
                        pix="yuv420p",
                        color=color,
                        container=c,
                        bitrate=_estimate_source_video_bitrate(profile),
                    ),
                    "hevc_videotoolbox-quality-vbr",
                )
            )

    if not candidates and not any_gpu_encoder_available():
        # Last resort only when this machine has zero hardware encode.
        candidates.append(
            (
                [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "17",
                    "-pix_fmt",
                    "yuv420p",
                    *color,
                ],
                "libx264-quality",
            )
        )

    log.info(
        "Quality encode candidates container=%s depth=%s codec=%s -> %s",
        c,
        depth,
        codec,
        [label for _, label in candidates],
    )
    return candidates


def lossless_video_encode_candidates(
    profile: SourceEncodeProfile,
    *,
    container: str,
    codec: VideoCodecPref = "auto",
) -> list[tuple[list[str], str]]:
    """
    Archive / master encode: keep source look (no mush, no extra noise), GPU NVENC,
    size near the source — not giant bit-exact FFV1/x264.
    """
    c = container.lower().lstrip(".")
    depth = bit_depth(profile)
    color = color_tag_args(profile)
    candidates: list[tuple[list[str], str]] = []

    # Archive on 10-bit HEVC masters: prefer HEVC so we do not crush to 8-bit H.264.
    effective: VideoCodecPref = codec
    is_hevc_10bit = depth >= 10 and (profile.vcodec or "").startswith("hevc")
    if is_hevc_10bit:
        if nvenc_hevc_available() and codec in {"h264_nvenc", "auto"}:
            effective = "hevc_nvenc"
        elif videotoolbox_hevc_available() and codec in {
            "h264_videotoolbox",
            "h264_nvenc",
            "auto",
        }:
            # h264_videotoolbox has no 10-bit pixel format at all.
            effective = "hevc_videotoolbox"

    src_br = _estimate_source_video_bitrate(profile)
    # Slight headroom vs source so re-encode does not add banding; hard cap vs balloon.
    if src_br:
        target = int(src_br * 1.06)
        maxrate = int(min(src_br * 1.22, max(src_br * 1.06, 100_000_000)))
    else:
        target = None
        maxrate = None

    for enc in _codec_order(effective, profile):
        if enc == "hevc_nvenc" and nvenc_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _nvenc_vbr_args(
                            "hevc_nvenc",
                            cq=15,
                            pix="p010le",
                            color=color,
                            preset="p7",
                            bitrate=target,
                            maxrate=maxrate,
                        ),
                        "hevc_nvenc-archive-10",
                    )
                )
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "hevc_nvenc",
                        cq=16,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "hevc_nvenc-archive",
                )
            )
        elif enc == "h264_nvenc" and nvenc_h264_available():
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "h264_nvenc",
                        cq=14,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "h264_nvenc-archive",
                )
            )
        elif enc == "av1_nvenc" and nvenc_av1_available():
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "av1_nvenc",
                        cq=17,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "av1_nvenc-archive",
                )
            )
        elif enc == "hevc_videotoolbox" and videotoolbox_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _videotoolbox_args(
                            "hevc_videotoolbox",
                            pix="p010le",
                            color=color,
                        container=c,
                            bitrate=target,
                            maxrate=maxrate,
                        ),
                        "hevc_videotoolbox-archive-10",
                    )
                )
            candidates.append(
                (
                    _videotoolbox_args(
                        "hevc_videotoolbox",
                        pix="yuv420p",
                        color=color,
                        container=c,
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "hevc_videotoolbox-archive",
                )
            )
        elif enc == "h264_videotoolbox" and videotoolbox_h264_available():
            candidates.append(
                (
                    _videotoolbox_args(
                        "h264_videotoolbox",
                        pix="yuv420p",
                        color=color,
                        container=c,
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "h264_videotoolbox-archive",
                )
            )

    if not candidates:
        raise RuntimeError(
            "Archive export needs a hardware encoder (NVENC or VideoToolbox). "
            "None found in ffmpeg."
        )

    log.info(
        "Archive encode candidates container=%s depth=%s codec=%s br=%s -> %s",
        c,
        depth,
        codec,
        src_br,
        [label for _, label in candidates],
    )
    return candidates


def audio_encode_args(
    profile: SourceEncodeProfile,
    *,
    container: str,
    mode: ExportMode = "quality",
) -> list[str]:
    """Copy when possible; otherwise AAC (avoid FLAC/PCM size blow-up on archive)."""
    del container  # container only mattered for old lossless PCM/FLAC paths
    if profile.acodec in _SAFE_AUDIO_COPY:
        return ["-c:a", "copy"]
    # quality + archive: transparent-enough AAC, not giant PCM/FLAC
    if mode in {"quality", "lossless"}:
        return ["-c:a", "aac", "-b:a", "320k"]
    return ["-c:a", "aac", "-b:a", "320k"]


def video_encode_args(profile: SourceEncodeProfile) -> tuple[list[str], str]:
    cands = quality_video_encode_candidates(profile, container="mp4")
    return cands[0]


def scale_filter_keep_res() -> str:
    return "scale=trunc(iw/2)*2:trunc(ih/2)*2"


def _parse_int(value: object) -> int | None:
    if value is None or value == "N/A":
        return None
    try:
        n = int(str(value))
    except (TypeError, ValueError):
        return None
    return n if n > 0 else None


def _parse_fps(value: object) -> float | None:
    if not value or value == "N/A" or value == "0/0":
        return None
    text = str(value)
    try:
        if "/" in text:
            num, den = text.split("/", 1)
            d = float(den)
            if d <= 0:
                return None
            return float(num) / d
        return float(text)
    except (TypeError, ValueError, ZeroDivisionError):
        return None
