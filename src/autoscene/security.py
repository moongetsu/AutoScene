"""Path / ID / token guards for the local AutoScene API."""

from __future__ import annotations

import os
import re
import secrets
from pathlib import Path

from autoscene.config import settings

PROJECT_ID_RE = re.compile(r"^[a-f0-9]{12}$")
SAFE_LABEL_RE = re.compile(r"[^A-Za-z0-9._ -]+")
VIDEO_EXTENSIONS = frozenset({".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v"})

_TOKEN: str | None = None


class SecurityError(ValueError):
    """Raised when a path or id fails a security check."""


def assert_project_id(project_id: str) -> str:
    if not PROJECT_ID_RE.fullmatch(project_id or ""):
        raise SecurityError("Invalid project id")
    return project_id


def safe_person_label(label: str | None, person_id: int) -> str:
    cleaned = SAFE_LABEL_RE.sub("", (label or "").strip())[:64].strip(" .")
    return cleaned or f"Person_{int(person_id)}"


def assert_under(path: Path, root: Path) -> Path:
    resolved = path.resolve()
    root_r = root.resolve()
    try:
        resolved.relative_to(root_r)
    except ValueError as exc:
        raise SecurityError("Path outside allowlist") from exc
    return resolved


def assert_video_file(path: Path) -> Path:
    resolved = path.resolve()
    if not resolved.is_file():
        raise SecurityError("Video path must be an existing file")
    if resolved.suffix.lower() not in VIDEO_EXTENSIONS:
        raise SecurityError("Unsupported video type")
    return resolved


def export_allow_roots(project_exports: Path) -> list[Path]:
    settings.ensure_dirs()
    roots = [
        project_exports.resolve(),
        (settings.data_dir / "exports").resolve(),
    ]
    home = Path.home()
    for name in ("Downloads", "Videos", "Documents", "Desktop"):
        candidate = home / name
        if candidate.is_dir():
            roots.append(candidate.resolve())
    return roots


def assert_export_output(path: Path, project_exports: Path) -> Path:
    """Allow writes under project exports, data_dir/exports, or common user folders."""
    resolved = path.expanduser().resolve()
    for root in export_allow_roots(project_exports):
        try:
            resolved.relative_to(root)
            return resolved
        except ValueError:
            continue
    raise SecurityError(
        "Export path must be under project exports, AutoScene exports, "
        "or Downloads/Videos/Documents/Desktop"
    )


def assert_openable_folder(path: Path) -> Path:
    """Reveal/open only under data_dir or common user media folders."""
    settings.ensure_dirs()
    resolved = path.expanduser().resolve()
    folder = resolved if resolved.is_dir() else resolved.parent
    roots = [settings.data_dir.resolve()]
    home = Path.home()
    for name in ("Downloads", "Videos", "Documents", "Desktop"):
        candidate = home / name
        if candidate.is_dir():
            roots.append(candidate.resolve())
    for root in roots:
        try:
            folder.relative_to(root)
            return folder
        except ValueError:
            continue
    raise SecurityError("Folder is outside the allowed open list")


def token_path() -> Path:
    settings.ensure_dirs()
    return settings.data_dir / ".api_token"


def ensure_api_token() -> str:
    """Return process API token; persist under data_dir for local UI bootstrap."""
    global _TOKEN
    if _TOKEN:
        return _TOKEN
    env = (os.environ.get("AUTOSCENE_API_TOKEN") or "").strip()
    if env:
        _TOKEN = env
        return _TOKEN
    path = token_path()
    if path.is_file():
        existing = path.read_text(encoding="utf-8").strip()
        if existing:
            _TOKEN = existing
            return _TOKEN
    _TOKEN = secrets.token_urlsafe(32)
    path.write_text(_TOKEN, encoding="utf-8")
    try:
        path.chmod(0o600)
    except OSError:
        pass
    return _TOKEN


def get_api_token() -> str:
    return ensure_api_token()


def safe_zip_extract(zip_path: Path, dest_root: Path) -> None:
    """Extract zip members only under dest_root (Zip Slip safe)."""
    import zipfile

    dest_root = dest_root.resolve()
    dest_root.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path, "r") as zf:
        for info in zf.infolist():
            name = info.filename.replace("\\", "/")
            if not name or name.endswith("/"):
                # directory entry
                target = (dest_root / name).resolve()
                try:
                    target.relative_to(dest_root)
                except ValueError as exc:
                    raise SecurityError(f"Blocked zip path: {info.filename}") from exc
                target.mkdir(parents=True, exist_ok=True)
                continue
            if name.startswith("/") or ".." in Path(name).parts:
                raise SecurityError(f"Blocked zip path: {info.filename}")
            target = (dest_root / name).resolve()
            try:
                target.relative_to(dest_root)
            except ValueError as exc:
                raise SecurityError(f"Blocked zip path: {info.filename}") from exc
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(info, "r") as src, target.open("wb") as out:
                while True:
                    chunk = src.read(1024 * 1024)
                    if not chunk:
                        break
                    out.write(chunk)
