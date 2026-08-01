from __future__ import annotations

import logging
import shutil
import subprocess
import threading
import uuid
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from fastapi import BackgroundTasks, FastAPI, File, Form, HTTPException, Request, UploadFile
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel, Field

from autoscene.ffmpeg_encode import VIDEO_CODEC_CHOICES, default_video_codec
from starlette.middleware.base import BaseHTTPMiddleware

from autoscene.config import settings
from autoscene.project import (
    PACK_CONTEXT_MODES,
    ClipOverride,
    ManualSegment,
    PackSettings,
    ProjectStore,
    SegmentExclusion,
    SegmentRecord,
    is_segment_excluded,
)
from autoscene.security import (
    SecurityError,
    assert_export_output,
    assert_openable_folder,
    assert_project_id,
    assert_under,
    assert_video_file,
    ensure_api_token,
    get_api_token,
    safe_person_label,
)

logging.basicConfig(level=logging.INFO)
log = logging.getLogger("autoscene.api")

# Keep import path light — Torch/CUDA warm in a background thread after bind.
settings.ensure_dirs()
store = ProjectStore()
_scan_locks: dict[str, threading.Lock] = {}
_scan_cancel: dict[str, threading.Event] = {}
_scan_pause: dict[str, threading.Event] = {}
_api_startup_ready = False
_api_startup_error: str | None = None
_api_startup_stage = "Starting…"
_warm_lock = threading.Lock()
_warm_started = False


def _cancel_event(project_id: str) -> threading.Event:
    return _scan_cancel.setdefault(project_id, threading.Event())


def _pause_event(project_id: str) -> threading.Event:
    """Event set ⇒ scan is paused (worker blocks until cleared)."""
    return _scan_pause.setdefault(project_id, threading.Event())

app = FastAPI(title="AutoScene", version="0.1.0")


class _ApiTokenMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request: Request, call_next):  # type: ignore[no-untyped-def]
        path = request.url.path
        if (
            request.method == "OPTIONS"
            or path == "/api/health"
            or not path.startswith("/api/")
        ):
            return await call_next(request)
        expected = get_api_token()
        got = (
            request.headers.get("x-autoscene-token")
            or request.headers.get("authorization", "").removeprefix("Bearer ").strip()
            or request.query_params.get("token")
            or ""
        )
        if not got or got != expected:
            return JSONResponse({"detail": "Unauthorized"}, status_code=401)
        return await call_next(request)


# Token first, CORS last → CORS is outermost and tags 401/errors for Vite+Tauri.
app.add_middleware(_ApiTokenMiddleware)
app.add_middleware(
    CORSMiddleware,
    allow_origins=[
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "tauri://localhost",
        "https://tauri.localhost",
    ],
    # Any local Vite / preview port (Tauri dev loads localhost:5173 → API on 8741).
    allow_origin_regex=r"https?://(localhost|127\.0\.0\.1)(:\d+)?$",
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["*"],
)
ensure_api_token()


@app.on_event("startup")
def _startup_warm_gpu() -> None:
    """Bind HTTP immediately; warm Torch/CUDA off the accept loop."""
    global _warm_started

    def warm() -> None:
        global _api_startup_ready, _api_startup_error, _api_startup_stage
        try:
            _api_startup_stage = "Loading graphics…"
            from autoscene.models_vault import ensure_torch_dll_path
            from autoscene.gpu import detect_gpu, ensure_cuda_dll_path

            ensure_torch_dll_path()
            ensure_cuda_dll_path()
            _api_startup_stage = "Checking graphics…"
            info = detect_gpu()
            if not info.available:
                _api_startup_error = info.detail or "Graphics unavailable"
                _api_startup_stage = "Graphics unavailable"
                log.error("GPU required but unavailable: %s", info.detail)
                return
            log.info("GPU ready: %s (%s)", info.name, info.detail)
            _api_startup_stage = "Starting workers…"
            _start_export_queue_worker()
            _api_startup_ready = True
            _api_startup_stage = "Ready"
            log.info("AutoScene API startup ready")
        except Exception as exc:  # noqa: BLE001
            _api_startup_error = str(exc)
            _api_startup_stage = "Startup failed"
            log.exception("GPU warm failed: %s", exc)

    with _warm_lock:
        if _warm_started:
            return
        _warm_started = True
    threading.Thread(target=warm, daemon=True, name="autoscene-gpu-warm").start()


def _require_ready() -> None:
    if _api_startup_ready:
        return
    if _api_startup_error:
        raise HTTPException(503, "Graphics are not ready. Check Settings, then restart.")
    raise HTTPException(503, "Still starting. Try again in a moment.")


def _start_export_queue_worker() -> None:
    from autoscene.cluster import Segment
    from autoscene.export import ExportCancelled, export_merged_video
    from autoscene.export_queue import ExportJob, export_queue

    def run_job(job: ExportJob, should_cancel) -> None:
        project = store.load(job.project_id)
        project.status = "exporting"
        project.progress = 0.0
        project.stage = f"{job.person_label} · Starting…"
        store.save(project)

        def on_progress(frac: float, stage: str) -> None:
            job.progress = max(0.0, min(1.0, float(frac)))
            job.stage = f"{job.person_label} · {stage}"
            try:
                p = store.load(job.project_id)
                p.status = "exporting"
                p.progress = job.progress
                p.stage = job.stage
                store.save(p)
            except FileNotFoundError:
                pass

        segs = [
            Segment(job.person_id, float(s["start_sec"]), float(s["end_sec"]))
            for s in job.segments
        ]
        try:
            written = export_merged_video(
                Path(project.video_path),
                segs,
                Path(job.output_path),
                reencode=job.reencode,
                on_progress=on_progress,
                export_mode=job.export_mode,
                video_codec=job.video_codec,  # type: ignore[arg-type]
                should_cancel=should_cancel,
            )
            job.output_path = str(written)
            job.progress = 1.0
            job.stage = f"{job.person_label} · Export complete"
            try:
                from datetime import datetime, timezone

                from autoscene.project import MergedExportRecord

                p = store.load(job.project_id)
                rec = MergedExportRecord(
                    id=job.id,
                    person_id=job.person_id,
                    person_label=job.person_label,
                    output_path=str(written),
                    scene_count=job.scene_count,
                    finished_at=datetime.now(timezone.utc).isoformat(),
                    export_mode=job.export_mode,
                    video_codec=job.video_codec,
                )
                rest = [
                    e
                    for e in p.merged_exports
                    if e.output_path != rec.output_path and e.id != rec.id
                ]
                p.merged_exports = [rec, *rest][:40]
                store.save(p)
            except FileNotFoundError:
                pass
        except ExportCancelled:
            raise
        finally:
            pending_left = len(export_queue.snapshot().get("pending") or [])
            if pending_left == 0:
                try:
                    p = store.load(job.project_id)
                    p.status = "ready"
                    p.progress = 0.0 if should_cancel() else 1.0
                    p.stage = ""
                    store.save(p)
                except FileNotFoundError:
                    pass

    export_queue.start(run_job)


class CreateProjectBody(BaseModel):
    name: str
    video_path: str
    backend: str = "insightface"
    model: str = "buffalo_l"


class ExportBody(BaseModel):
    person_id: int
    reencode: bool = False
    export_mode: str | None = None  # copy | quality | lossless
    # nvenc on Windows/Linux, videotoolbox on macOS | auto
    video_codec: str = Field(default_factory=default_video_codec)


class MergedExportBody(BaseModel):
    person_id: int
    output_path: str
    reencode: bool = False
    export_mode: str | None = None
    video_codec: str = Field(default_factory=default_video_codec)


class ToggleSegmentBody(BaseModel):
    person_id: int
    start_sec: float
    end_sec: float
    excluded: bool


class ManualSegmentBody(BaseModel):
    start_sec: float
    end_sec: float
    person_ids: list[int] = Field(default_factory=list)
    label: str = ""


class ManualSegmentPatchBody(BaseModel):
    start_sec: float | None = None
    end_sec: float | None = None
    person_ids: list[int] | None = None
    label: str | None = None


class RenameBody(BaseModel):
    label: str


class MergePeopleBody(BaseModel):
    keep_person_id: int
    merge_person_ids: list[int]


class OpenFolderBody(BaseModel):
    path: str


class PackSettingsBody(BaseModel):
    context_mode: str | None = None
    lead_shots: int | None = None
    tail_shots: int | None = None
    max_clip_sec: float | None = None


class ClipOverrideBody(BaseModel):
    shot_index: int
    # include | exclude | clear
    action: str = "include"


@app.get("/api/health")
def health(request: Request) -> dict[str, Any]:
    body: dict[str, Any] = {
        "ok": True,
        "ready": _api_startup_ready,
        "stage": _api_startup_stage,
        "error": _api_startup_error,
        "version": "0.1.0",
        "auth": True,
    }
    host = request.client.host if request.client else ""
    if host in ("127.0.0.1", "::1", "localhost"):
        body["token"] = get_api_token()
    return body


@app.get("/api/gpu")
def gpu() -> dict[str, Any]:
    if not _api_startup_ready:
        return {
            "available": False,
            "name": None,
            "provider": "pending",
            "detail": _api_startup_error or _api_startup_stage,
        }
    from autoscene.gpu import detect_gpu

    info = detect_gpu()
    return {
        "available": info.available,
        "name": info.name,
        "provider": info.provider,
        "detail": info.detail,
    }


@app.get("/api/export/capabilities")
def export_capabilities() -> dict[str, Any]:
    from autoscene.ffmpeg_encode import available_export_codecs

    caps = available_export_codecs()
    return {
        "codecs": caps,
        "modes": ["quality", "lossless", "copy"],
        "default_mode": "quality",
        "default_codec": default_video_codec(),
    }


class UpdateSettingsBody(BaseModel):
    sample_fps: float | None = None
    det_size: int | None = None
    detect_max_edge: int | None = None
    min_face_size: int | None = None
    cluster_min_size: int | None = None
    cluster_merge_sim: float | None = None
    content_preset: str | None = None
    prefetch_depth: int | None = None
    face_select_mode: str | None = None
    face_relative_pct: float | None = None
    face_top_n: int | None = None
    default_backend: str | None = None
    default_model: str | None = None


@app.get("/api/settings")
def get_settings() -> dict[str, Any]:
    return settings.public_dict()


@app.patch("/api/settings")
def patch_settings(body: UpdateSettingsBody) -> dict[str, Any]:
    """Persist scan knobs (apply to the next scan start)."""
    patch = body.model_dump(exclude_none=True)
    if not patch:
        return settings.public_dict()
    return settings.apply_runtime(patch, persist=True)


class InstallModelsBody(BaseModel):
    targets: list[str] | None = None


@app.get("/api/models/status")
def models_status() -> dict[str, Any]:
    from autoscene.models_vault import vault_status

    return vault_status()


@app.post("/api/models/install")
def models_install(body: InstallModelsBody | None = None) -> dict[str, Any]:
    """Background install of deps + face packs (Model Vault)."""
    from autoscene.models_vault import start_install, vault_status

    # None / omitted → install everything. Explicit [] → no-op status only.
    if body is None or body.targets is None:
        targets = ["all"]
    elif len(body.targets) == 0:
        return vault_status()
    else:
        targets = body.targets
    start_install(targets)
    return vault_status()


@app.get("/api/models/install/status")
def models_install_status() -> dict[str, Any]:
    from autoscene.models_vault import current_job, vault_status

    st = vault_status()
    st["job"] = current_job().to_dict()
    return st


@app.get("/api/projects")
def list_projects() -> list[dict[str, Any]]:
    return [p.to_api_dict() for p in store.list_projects()]


@app.post("/api/projects")
def create_project(body: CreateProjectBody) -> dict[str, Any]:
    try:
        path = assert_video_file(Path(body.video_path))
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.create(body.name, path, body.backend, body.model)
    return project.to_api_dict()


@app.post("/api/projects/upload")
async def upload_project(
    background_tasks: BackgroundTasks,
    file: UploadFile = File(...),
    name: str = Form("Untitled"),
    backend: str = Form("insightface"),
    model: str = Form("buffalo_l"),
    autostart: bool = Form(True),
) -> dict[str, Any]:
    settings.ensure_dirs()
    uploads = settings.data_dir / "uploads"
    uploads.mkdir(parents=True, exist_ok=True)
    suffix = Path(file.filename or "video.mp4").suffix.lower() or ".mp4"
    if suffix not in {".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v"}:
        raise HTTPException(400, "Unsupported video type")
    dest = uploads / f"{Path(file.filename or 'video').stem}_{Path(file.filename or '').stem}"
    # unique
    import uuid

    dest = uploads / f"{uuid.uuid4().hex[:10]}{suffix}"
    max_bytes = 50 * 1024 * 1024 * 1024
    written = 0
    try:
        with dest.open("wb") as out:
            while True:
                chunk = file.file.read(1024 * 1024)
                if not chunk:
                    break
                written += len(chunk)
                if written > max_bytes:
                    raise HTTPException(413, "Upload too large")
                out.write(chunk)
    except HTTPException:
        dest.unlink(missing_ok=True)
        raise
    except Exception:
        dest.unlink(missing_ok=True)
        raise
    project = store.create(name or dest.stem, dest, backend, model)
    if autostart:
        _cancel_event(project.id).clear()
        project.status = "cutting"
        project.progress = 0.0
        project.stage = "Queued · scenes"
        project.progress_phase = "cuts"
        store.save(project)
        background_tasks.add_task(_scan_job, project.id)
    return project.to_api_dict()


@app.get("/api/projects/{project_id}")
def get_project(project_id: str) -> dict[str, Any]:
    try:
        return store.load(project_id).to_api_dict()
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc


@app.post("/api/projects/{project_id}/scan")
def start_scan(project_id: str, background_tasks: BackgroundTasks) -> dict[str, Any]:
    _require_ready()
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if project.status in ("scanning", "cutting"):
        return project.to_api_dict()
    if project.status == "paused":
        raise HTTPException(409, "Scan is paused. Call /scan/resume to continue.")
    # Align project face stack with current content preset / defaults.
    if settings.default_backend:
        project.backend = str(settings.default_backend)
    if settings.default_model:
        project.model = str(settings.default_model)
    log.info(
        "scan_enqueue project=%s backend=%s model=%s sample_fps=%s det_size=%s "
        "detect_max_edge=%s content_preset=%s face_select=%s cut=%s",
        project_id,
        project.backend,
        project.model,
        settings.sample_fps,
        settings.det_size,
        getattr(settings, "detect_max_edge", None),
        settings.content_preset,
        settings.face_select_mode,
        settings.scene_cut_detector,
    )
    _cancel_event(project_id).clear()
    _pause_event(project_id).clear()
    # Fresh scan always re-cuts (settings may have changed).
    project.shots = []
    project.scene_cut_detector_used = ""
    project.status = "cutting"
    project.progress = 0.0
    project.stage = "Queued · scenes"
    project.progress_phase = "cuts"
    project.error = None
    store.save(project)
    background_tasks.add_task(_scan_job, project_id)
    return project.to_api_dict()


@app.get("/api/projects/{project_id}/scan-debug")
def get_scan_debug(project_id: str, lines: int = 200) -> dict[str, Any]:
    """Tail per-scan debug log (settings dump, backend, detect ticks)."""
    from autoscene.scan_debug import scan_log_path, tail_scan_log

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    tail = tail_scan_log(project_id, max_lines=max(20, min(2000, int(lines))))
    return {
        "project_id": project_id,
        "status": project.status,
        "stage": project.stage,
        "backend": project.backend,
        "model": project.model,
        "frames_done": project.frames_done,
        "frames_total": project.frames_total,
        "detect_hits": project.detect_hits,
        "error": project.error,
        "runtime_settings": {
            "sample_fps": settings.sample_fps,
            "det_size": settings.det_size,
            "detect_max_edge": getattr(settings, "detect_max_edge", None),
            "min_face_size": settings.min_face_size,
            "cluster_merge_sim": settings.cluster_merge_sim,
            "cluster_min_size": settings.cluster_min_size,
            "content_preset": settings.content_preset,
            "prefetch_depth": settings.prefetch_depth,
            "face_select_mode": settings.face_select_mode,
            "face_relative_pct": settings.face_relative_pct,
            "default_backend": settings.default_backend,
            "default_model": settings.default_model,
        },
        "log_path": str(scan_log_path(project_id)),
        **tail,
    }


@app.post("/api/projects/{project_id}/scan/pause")
def pause_scan(project_id: str) -> dict[str, Any]:
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if project.status != "scanning":
        return project.to_api_dict()
    _pause_event(project_id).set()
    project.status = "paused"
    project.stage = "Paused"
    project.eta_sec = None
    store.save(project)
    return project.to_api_dict()


@app.post("/api/projects/{project_id}/scan/resume")
def resume_scan(project_id: str) -> dict[str, Any]:
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if project.status != "paused":
        return project.to_api_dict()
    lock = _scan_locks.get(project_id)
    if lock is None or not lock.locked():
        # Worker died while paused — client should start a new scan
        _pause_event(project_id).clear()
        _cancel_event(project_id).clear()
        project.status = "idle"
        project.stage = "Stopped"
        store.save(project)
        raise HTTPException(
            409,
            "Scan worker is not running. Start a new scan to continue.",
        )
    _pause_event(project_id).clear()
    project.status = "scanning"
    project.stage = "Resuming…"
    store.save(project)
    return project.to_api_dict()


@app.post("/api/projects/{project_id}/scan/stop")
def stop_scan(project_id: str) -> dict[str, Any]:
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if project.status not in ("scanning", "paused"):
        return project.to_api_dict()
    _pause_event(project_id).clear()
    _cancel_event(project_id).set()
    project.stage = "Stopping…"
    if project.status == "paused":
        project.status = "scanning"  # worker still owns the job until cancel lands
    store.save(project)
    return project.to_api_dict()


class DeleteProjectBody(BaseModel):
    confirm_stop: bool = False


@app.delete("/api/projects/{project_id}")
def delete_project(
    project_id: str,
    confirm_stop: bool = False,
) -> dict[str, Any]:
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc

    was_scanning = project.status in ("scanning", "paused")
    lock = _scan_locks.get(project_id)
    scan_running = lock is not None and lock.locked()

    if (was_scanning or scan_running) and not confirm_stop:
        raise HTTPException(
            409,
            "Project is scanning. Confirm stop (confirm_stop=true) before delete.",
        )

    # Tombstone immediately so in-flight scan saves cannot recreate the folder
    store._deleted.add(project_id)
    _pause_event(project_id).clear()
    _cancel_event(project_id).set()

    if was_scanning or scan_running:
        lock = _scan_locks.get(project_id)
        if lock is not None:
            acquired = lock.acquire(timeout=180)
            if acquired:
                lock.release()
            else:
                # Still force-delete; tombstone keeps saves from resurrecting it
                log.warning("Scan lock wait timed out for delete project=%s", project_id)

    try:
        store.delete(project_id)
    except FileNotFoundError:
        pass
    except OSError as exc:
        # Tombstone stays — project stays hidden even if files locked briefly
        log.warning("Delete rmtree incomplete project=%s: %s", project_id, exc)
    except ValueError as exc:
        store._deleted.discard(project_id)
        raise HTTPException(400, str(exc)) from exc
    finally:
        _scan_cancel.pop(project_id, None)
        lock = _scan_locks.get(project_id)
        if lock is None or not lock.locked():
            _scan_locks.pop(project_id, None)

    return {"ok": True, "id": project_id, "stopped_scan": was_scanning or scan_running}


@app.patch("/api/projects/{project_id}/people/{person_id}")
def rename_person(project_id: str, person_id: int, body: RenameBody) -> dict[str, Any]:
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    for person in project.people:
        if person.person_id == person_id:
            person.label = safe_person_label(body.label, person_id)
            store.save(project)
            return project.to_api_dict()
    raise HTTPException(404, "Person not found")


@app.post("/api/projects/{project_id}/cast/teach")
async def teach_cast_endpoint(
    project_id: str,
    files: list[UploadFile] = File(...),
    mode: str = Form("match"),
    label: str | None = Form(None),
    person_id: str | None = Form(None),
) -> dict[str, Any]:
    """
    Teach cast from reference stills.
    mode=pin|match|pack — see cast_teach.teach_cast.
    """
    from autoscene.cast_teach import matches_to_dict, teach_cast

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc

    images: list[tuple[str, bytes]] = []
    if len(files) > 10:
        raise HTTPException(400, "Too many images (max 10)")
    for f in files:
        data = await f.read()
        if not data:
            continue
        if len(data) > 20 * 1024 * 1024:
            raise HTTPException(413, "Image too large (max 20MB)")
        name = (f.filename or "ref.jpg").lower()
        if not name.endswith((".jpg", ".jpeg", ".png", ".webp", ".bmp")):
            raise HTTPException(400, "Unsupported image type")
        images.append((f.filename or "ref.jpg", data))
    if not images:
        raise HTTPException(400, "No image data")

    pid: int | None = None
    if person_id is not None and str(person_id).strip() != "":
        try:
            pid = int(person_id)
        except (TypeError, ValueError) as exc:
            raise HTTPException(400, "Invalid person_id") from exc

    try:
        project, matches = teach_cast(
            store,
            project,
            images=images,
            mode=mode,
            label=label,
            person_id=pid,
        )
    except ValueError as exc:
        raise HTTPException(400, str(exc)) from exc
    except Exception as exc:  # noqa: BLE001
        log.exception("cast teach failed project=%s", project_id)
        raise HTTPException(500, f"Cast teach failed: {exc}") from exc

    store.save(project)
    return {
        "project": project.to_api_dict(),
        "matches": matches_to_dict(matches),
    }


@app.post("/api/projects/{project_id}/people/merge")
def merge_people_endpoint(project_id: str, body: MergePeopleBody) -> dict[str, Any]:
    """Absorb other cast members into keep_person_id; scenes stay chronological."""
    from autoscene.cast_ops import merge_people

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if project.status != "ready":
        raise HTTPException(409, "Merge is only available when the project is ready.")
    try:
        merge_people(project, body.keep_person_id, body.merge_person_ids)
    except KeyError as exc:
        raise HTTPException(404, str(exc)) from exc
    except ValueError as exc:
        raise HTTPException(400, str(exc)) from exc
    store.save(project)
    return project.to_api_dict()


@app.patch("/api/projects/{project_id}/segments/exclusion")
def toggle_segment_exclusion(
    project_id: str,
    body: ToggleSegmentBody,
) -> dict[str, Any]:
    project = store.load(project_id)
    key = (body.person_id, round(body.start_sec, 4), round(body.end_sec, 4))
    project.excluded_segments = [
        ex
        for ex in project.excluded_segments
        if (ex.person_id, round(ex.start_sec, 4), round(ex.end_sec, 4)) != key
    ]
    if body.excluded:
        project.excluded_segments.append(
            SegmentExclusion(
                person_id=body.person_id,
                start_sec=body.start_sec,
                end_sec=body.end_sec,
            )
        )
    store.save(project)
    return project.to_api_dict()


def _clamp_manual_range(
    project, start_sec: float, end_sec: float
) -> tuple[float, float]:
    limit = float(project.duration_sec or 0.0)
    a, b = float(start_sec), float(end_sec)
    if b < a:
        a, b = b, a
    a = max(0.0, a)
    if limit > 0:
        a = min(a, limit)
        b = min(b, limit)
    if b - a < 0.04:
        raise HTTPException(400, "Range is too short")
    return a, b


def _valid_person_ids(project, person_ids: list[int]) -> list[int]:
    """
    Validate cast tags, rejecting unknown ids rather than dropping them.

    Silently discarding an id would report "range added to 2 packs" while
    the range landed in none — the client only ever sends ids from the
    cast it just rendered, so a mismatch is a real desync worth surfacing.
    """
    known = {p.person_id for p in project.people}
    wanted = {int(pid) for pid in person_ids}
    unknown = sorted(wanted - known)
    if unknown:
        raise HTTPException(400, f"Unknown cast member(s): {unknown}")
    return sorted(wanted)


@app.post("/api/projects/{project_id}/manual-segments")
def create_manual_segment(project_id: str, body: ManualSegmentBody) -> dict[str, Any]:
    """Add a user-drawn range, tagged to zero or more cast members."""
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    start, end = _clamp_manual_range(project, body.start_sec, body.end_sec)
    project.manual_segments.append(
        ManualSegment(
            id=uuid.uuid4().hex[:12],
            start_sec=start,
            end_sec=end,
            person_ids=_valid_person_ids(project, body.person_ids),
            label=body.label.strip()[:120],
            created_at=datetime.now(timezone.utc).isoformat(),
        )
    )
    project.manual_segments.sort(key=lambda m: m.start_sec)
    store.save(project)
    return project.to_api_dict()


@app.patch("/api/projects/{project_id}/manual-segments/{segment_id}")
def update_manual_segment(
    project_id: str, segment_id: str, body: ManualSegmentPatchBody
) -> dict[str, Any]:
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    target = next((m for m in project.manual_segments if m.id == segment_id), None)
    if target is None:
        raise HTTPException(404, "Manual segment not found")
    start = body.start_sec if body.start_sec is not None else target.start_sec
    end = body.end_sec if body.end_sec is not None else target.end_sec
    target.start_sec, target.end_sec = _clamp_manual_range(project, start, end)
    if body.person_ids is not None:
        target.person_ids = _valid_person_ids(project, body.person_ids)
    if body.label is not None:
        target.label = body.label.strip()[:120]
    project.manual_segments.sort(key=lambda m: m.start_sec)
    store.save(project)
    return project.to_api_dict()


@app.delete("/api/projects/{project_id}/manual-segments/{segment_id}")
def delete_manual_segment(project_id: str, segment_id: str) -> dict[str, Any]:
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    before = len(project.manual_segments)
    project.manual_segments = [m for m in project.manual_segments if m.id != segment_id]
    if len(project.manual_segments) == before:
        raise HTTPException(404, "Manual segment not found")
    store.save(project)
    return project.to_api_dict()


def _require_person(project, person_id: int):
    person = next(
        (p for p in project.people if int(p.person_id) == int(person_id)), None
    )
    if person is None:
        raise HTTPException(404, f"Unknown cast member: {person_id}")
    return person


@app.patch("/api/projects/{project_id}/packs/{person_id}")
def update_pack_settings(
    project_id: str, person_id: int, body: PackSettingsBody
) -> dict[str, Any]:
    """
    Set how much context this character's pack keeps around each appearance.

    Applies immediately — clips are widened when a pack is read, so changing
    mode never needs a rescan.
    """
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    _require_person(project, person_id)

    row = next(
        (p for p in project.pack_settings if int(p.person_id) == int(person_id)),
        None,
    )
    if row is None:
        row = PackSettings(**asdict(project.pack_settings_for(person_id)))
        project.pack_settings.append(row)

    if body.context_mode is not None:
        mode = body.context_mode.strip().lower()
        if mode not in PACK_CONTEXT_MODES:
            raise HTTPException(
                400, f"context_mode must be one of {list(PACK_CONTEXT_MODES)}"
            )
        if mode in ("conversation", "scene") and not project.scene_groups:
            raise HTTPException(
                409,
                "This project has no scene grouping yet. Rebuild scenes first.",
            )
        row.context_mode = mode
    if body.lead_shots is not None:
        row.lead_shots = max(0, min(10, int(body.lead_shots)))
    if body.tail_shots is not None:
        row.tail_shots = max(0, min(10, int(body.tail_shots)))
    if body.max_clip_sec is not None:
        row.max_clip_sec = max(1.0, float(body.max_clip_sec)) or None

    store.save(project)
    return {
        "project": project.to_api_dict(),
        "clips": _clips_payload(project, person_id),
    }


@app.post("/api/projects/{project_id}/packs/{person_id}/overrides")
def set_clip_override(
    project_id: str, person_id: int, body: ClipOverrideBody
) -> dict[str, Any]:
    """Pull a missed shot into the pack, drop one that does not fit, or reset."""
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    _require_person(project, person_id)

    action = body.action.strip().lower()
    if action not in ("include", "exclude", "clear"):
        raise HTTPException(400, "action must be include, exclude or clear")
    shot_index = int(body.shot_index)
    if not (0 <= shot_index < len(project.shots)):
        raise HTTPException(404, f"No shot at index {shot_index}")

    project.clip_overrides = [
        o
        for o in project.clip_overrides
        if not (
            int(o.person_id) == int(person_id) and int(o.shot_index) == shot_index
        )
    ]
    if action != "clear":
        project.clip_overrides.append(
            ClipOverride(
                person_id=int(person_id), shot_index=shot_index, action=action
            )
        )
    store.save(project)
    return {
        "project": project.to_api_dict(),
        "clips": _clips_payload(project, person_id),
    }


@app.post("/api/projects/{project_id}/scene-groups/rebuild")
def rebuild_scene_groups(project_id: str) -> dict[str, Any]:
    """
    Regroup shots into scenes from the current cast.

    Also the migration path for projects scanned before grouping existed: their
    fingerprints are missing, so grouping falls back to cast continuity, which
    is weaker on faceless establishing shots but still unlocks the wider modes.
    """
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    if project.status != "ready":
        raise HTTPException(409, "Scenes can only be rebuilt on a ready project.")
    if not project.shots:
        raise HTTPException(409, "This project has no scene cuts to group.")

    from autoscene.scene_cuts import ShotSpan
    from autoscene.scene_groups import build_scene_groups
    from autoscene.shot_fingerprint import load_fingerprints

    spans = [
        ShotSpan(float(s.start_sec), float(s.end_sec)) for s in project.shots
    ]
    project.scene_groups = build_scene_groups(
        spans,
        project.segments,
        fingerprints=load_fingerprints(store._dir(project_id)),
    )
    store.save(project)
    return project.to_api_dict()


@app.get("/api/projects/{project_id}/packs/{person_id}/clips")
def get_pack_clips(project_id: str, person_id: int) -> dict[str, Any]:
    """
    The clips this character's pack will export, at the current context mode.

    Served per-person on demand rather than folded into the project payload,
    which the UI polls about once a second.
    """
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    _require_person(project, person_id)
    return _clips_payload(project, person_id)


def _clips_payload(project, person_id: int) -> dict[str, Any]:
    settings_row = project.pack_settings_for(person_id)
    clips = _active_segments(project, person_id)
    shot_starts = [float(sh.start_sec) for sh in project.shots]

    covered: set[int] = set()
    for c in clips:
        for i, s in enumerate(shot_starts):
            if c.start_sec <= s < c.end_sec:
                covered.add(i)

    # Shots holding a detection of this character that the pack does not
    # currently ship — the "you're missing this" hint the timeline renders.
    detected_shots: set[int] = set()
    for seg in project.segments:
        if int(seg.person_id) != int(person_id):
            continue
        for i, s in enumerate(shot_starts):
            if abs(s - float(seg.start_sec)) <= 0.06:
                detected_shots.add(i)
                break

    return {
        "person_id": int(person_id),
        "context_mode": settings_row.context_mode,
        "lead_shots": settings_row.lead_shots,
        "tail_shots": settings_row.tail_shots,
        "max_clip_sec": settings_row.max_clip_sec,
        "clips": [asdict(c) for c in clips],
        "covered_shot_indices": sorted(covered),
        "missing_shot_indices": sorted(detected_shots - covered),
        "overrides": [
            asdict(o)
            for o in project.clip_overrides
            if int(o.person_id) == int(person_id)
        ],
    }


def _person_segments(project, person_id: int) -> list[SegmentRecord]:
    segs = [s for s in project.segments if s.person_id == person_id]
    segs.sort(key=lambda s: s.start_sec)
    return segs


def _manual_segments_for(project, person_id: int) -> list[SegmentRecord]:
    """User-drawn ranges tagged with this person, as plain segments."""
    out: list[SegmentRecord] = []
    for ms in getattr(project, "manual_segments", None) or []:
        if person_id not in (ms.person_ids or []):
            continue
        out.append(
            SegmentRecord(
                person_id=person_id,
                start_sec=float(ms.start_sec),
                end_sec=float(ms.end_sec),
            )
        )
    return out


def _active_segments(project, person_id: int) -> list[SegmentRecord]:
    """
    The clips this character's pack actually ships.

    One choke point for everything that reads a pack — /export, /export/merged
    and /packs/{id}/clips all route through here, so context mode, user
    overrides and exclusions can never disagree between preview and export.

    Order matters: widen for context FIRST, then drop what the user excluded.
    Excluding first would let a removed shot come back as context for a
    neighbouring one.
    """
    from autoscene.cluster import Segment, coalesce_segments
    from autoscene.pack_context import expand_segments

    settings_row = project.pack_settings_for(person_id)
    overrides = [
        o
        for o in getattr(project, "clip_overrides", None) or []
        if int(o.person_id) == int(person_id)
    ]
    includes = {int(o.shot_index) for o in overrides if o.action == "include"}
    excludes = {int(o.shot_index) for o in overrides if o.action == "exclude"}

    detected = expand_segments(
        _person_segments(project, person_id),
        person_id=person_id,
        shots=project.shots,
        scene_groups=project.scene_groups,
        cocast_edges=project.cocast_edges(),
        all_segments=project.segments,
        mode=settings_row.context_mode,
        lead_shots=settings_row.lead_shots,
        tail_shots=settings_row.tail_shots,
        max_clip_sec=settings_row.max_clip_sec,
        extra_shot_indices=includes,
        excluded_shot_indices=excludes,
    )

    # Manual ranges join the pack here, so both /export and /export/merged
    # pick them up. They are user-drawn verbatim and never widened.
    candidates = [
        SegmentRecord(
            person_id=s.person_id,
            start_sec=s.start_sec,
            end_sec=s.end_sec,
            preview_start_sec=s.preview_start_sec,
            preview_end_sec=s.preview_end_sec,
            face_time_sec=s.face_time_sec,
        )
        for s in detected
    ] + _manual_segments_for(project, person_id)

    active = [
        s
        for s in candidates
        if not is_segment_excluded(
            project.excluded_segments,
            s.person_id,
            s.start_sec,
            s.end_sec,
        )
    ]
    active.sort(key=lambda s: s.start_sec)
    coalesced = coalesce_segments(
        [
            Segment(
                s.person_id,
                s.start_sec,
                s.end_sec,
                preview_start_sec=s.preview_start_sec,
                preview_end_sec=s.preview_end_sec,
                face_time_sec=s.face_time_sec,
            )
            for s in active
        ],
        gap_sec=0.0,
        same_plate_only=True,
    )
    return [
        SegmentRecord(
            person_id=s.person_id,
            start_sec=s.start_sec,
            end_sec=s.end_sec,
            preview_start_sec=s.preview_start_sec,
            preview_end_sec=s.preview_end_sec,
            face_time_sec=s.face_time_sec,
        )
        for s in coalesced
    ]


@app.post("/api/projects/{project_id}/export")
def export_person(project_id: str, body: ExportBody) -> dict[str, Any]:
    _require_ready()
    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    project = store.load(project_id)
    # Honor user exclusions — per-clip export used to ship clips the user
    # had already switched off in the grid.
    segs = _active_segments(project, body.person_id)
    if not segs:
        raise HTTPException(400, "No segments for person")
    person = next((p for p in project.people if p.person_id == body.person_id), None)
    label = safe_person_label(person.label if person else None, body.person_id)
    exports_root = store.exports_dir(project_id)
    out_dir = assert_under(exports_root / f"person_{body.person_id}_{label}", exports_root)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    from autoscene.cluster import Segment
    from autoscene.export import export_person_segments

    written = export_person_segments(
        Path(project.video_path),
        [Segment(s.person_id, s.start_sec, s.end_sec) for s in segs],
        out_dir,
        reencode=body.reencode,
        export_mode=body.export_mode,
        video_codec=body.video_codec,  # type: ignore[arg-type]
    )
    return {
        "ok": True,
        "out_dir": str(out_dir),
        "clips": [str(p) for p in written],
        "count": len(written),
    }


@app.post("/api/projects/{project_id}/export/merged")
def export_merged(project_id: str, body: MergedExportBody) -> dict[str, Any]:
    """Enqueue a merged export job (non-blocking). Poll /export/queue for progress."""
    _require_ready()
    from autoscene.export import prefer_output_path, resolve_export_mode
    from autoscene.export_queue import ExportJob, export_queue, new_job_id

    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc

    project = store.load(project_id)
    segs = _active_segments(project, body.person_id)
    if not segs:
        raise HTTPException(400, "No active scenes to export")
    person = next((p for p in project.people if p.person_id == body.person_id), None)
    label = safe_person_label(person.label if person else None, body.person_id)
    output = Path(body.output_path)

    video_path = Path(project.video_path)
    mode = resolve_export_mode(
        video_path,
        export_mode=body.export_mode,
        reencode=body.reencode,
    )
    codec = (
        body.video_codec
        if body.video_codec in VIDEO_CODEC_CHOICES
        else default_video_codec()
    )
    if output.suffix.lower() not in {".mp4", ".mov", ".mkv"}:
        output = prefer_output_path(output, mode=mode, codec=codec)  # type: ignore[arg-type]
    if not output.name or output.name in {".", ".."}:
        suf = prefer_output_path(
            Path(f"{label}_scenes"), mode=mode, codec=codec  # type: ignore[arg-type]
        ).suffix
        output = output / f"{label}_scenes{suf}"
    output = prefer_output_path(output, mode=mode, codec=codec)  # type: ignore[arg-type]
    try:
        output = assert_export_output(output, store.exports_dir(project_id))
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc

    job = ExportJob(
        id=new_job_id(),
        project_id=project_id,
        person_id=int(body.person_id),
        person_label=label,
        output_path=str(output),
        export_mode=mode,
        video_codec=codec,
        reencode=bool(body.reencode),
        segments=[
            {"start_sec": float(s.start_sec), "end_sec": float(s.end_sec)}
            for s in segs
        ],
        scene_count=len(segs),
        stage="Queued",
    )
    export_queue.enqueue(job)

    # Mark project exporting if not already
    try:
        project = store.load(project_id)
        if project.status != "exporting":
            project.status = "exporting"
            project.progress = 0.0
            project.stage = f"Queued · {label}"
            store.save(project)
    except FileNotFoundError:
        pass

    snap = export_queue.snapshot()
    return {
        "ok": True,
        "queued": True,
        "job_id": job.id,
        "person_label": label,
        "scene_count": len(segs),
        "output_path": str(output),
        "queue": snap,
    }


@app.get("/api/projects/{project_id}/export/queue")
def export_queue_status(project_id: str) -> dict[str, Any]:
    from autoscene.export_queue import export_queue

    _ = store.load(project_id)  # 404 if missing
    snap = export_queue.snapshot()
    current = snap.get("current")
    if current and current.get("project_id") != project_id:
        current = None
    pending = [
        j for j in (snap.get("pending") or []) if j.get("project_id") == project_id
    ]
    recent = [
        j for j in (snap.get("recent") or []) if j.get("project_id") == project_id
    ]
    return {
        "active": bool(current) or bool(pending),
        "current": current,
        "pending": pending,
        "recent": recent,
        "pending_count": len(pending),
        "global_busy": bool(snap.get("current")) or bool(snap.get("pending")),
    }


@app.post("/api/projects/{project_id}/export/queue/cancel")
def export_queue_cancel(project_id: str) -> dict[str, Any]:
    """Cancel export jobs for this project only."""
    from autoscene.export_queue import export_queue

    try:
        assert_project_id(project_id)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc

    result = export_queue.cancel_for_project(project_id)
    try:
        project = store.load(project_id)
        project.status = "ready"
        project.stage = ""
        project.progress = 0.0
        store.save(project)
    except FileNotFoundError:
        pass
    return {**result, "queue": export_queue_status(project_id)}


@app.get("/api/projects/{project_id}/video")
def project_video(project_id: str) -> FileResponse:
    """
    Browser-playable video for Gallery reel / Import preview.
    MKV/MOV etc. are remuxed once to projects/<id>/browser_proxy.mp4 (Chromium
    cannot play most .mkv sources: "no supported sources").
    """
    from autoscene.video_proxy import ensure_browser_playable

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    video = Path(project.video_path)
    if not video.exists():
        raise HTTPException(404, "Video missing")
    try:
        playable = ensure_browser_playable(video, store._dir(project_id))
    except FileNotFoundError as exc:
        raise HTTPException(404, "Video missing") from exc
    except RuntimeError as exc:
        raise HTTPException(500, str(exc)) from exc
    except Exception as exc:  # noqa: BLE001
        log.exception("browser video proxy failed project=%s", project_id)
        raise HTTPException(500, f"Could not prepare video for browser: {exc}") from exc

    suffix = playable.suffix.lower()
    media = {
        ".mp4": "video/mp4",
        ".m4v": "video/mp4",
        ".webm": "video/webm",
    }.get(suffix, "video/mp4")
    # Range support required for HTML5 seek/play. inline so <video> plays, not downloads.
    return FileResponse(
        playable,
        media_type=media,
        filename=playable.name,
        content_disposition_type="inline",
    )


@app.get("/api/projects/{project_id}/video-proxy-status")
def project_video_proxy_status(project_id: str) -> dict[str, Any]:
    """Whether a Chromium-safe H.264/AAC (etc.) proxy exists."""
    from autoscene.video_proxy import proxy_status

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    video = Path(project.video_path)
    return proxy_status(video, store._dir(project_id))


@app.get("/api/projects/{project_id}/scene-frame")
def project_scene_frame(
    project_id: str,
    t: float = 0.0,
    person_id: int | None = None,
) -> FileResponse:
    """JPEG still for Export scene tiles.

    When ``person_id`` is set and detection geometry exists, returns a
    focus-person face crop so reverse-shot plates never mislabel the tile.
    """
    from autoscene.scene_frame import ensure_scene_frame

    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    video = Path(project.video_path)
    if not video.exists():
        raise HTTPException(404, "Video missing")
    try:
        path = ensure_scene_frame(
            video,
            store._dir(project_id),
            t,
            person_id=person_id,
        )
    except FileNotFoundError as exc:
        raise HTTPException(404, "Video missing") from exc
    except RuntimeError as exc:
        raise HTTPException(500, str(exc)) from exc
    except Exception as exc:  # noqa: BLE001
        log.exception("scene frame failed project=%s t=%s", project_id, t)
        raise HTTPException(500, f"Could not extract scene frame: {exc}") from exc
    return FileResponse(
        path,
        media_type="image/jpeg",
        filename=path.name,
        content_disposition_type="inline",
        headers={"Cache-Control": "private, max-age=86400"},
    )


_video_probe_cache: dict[tuple[str, int, int], dict[str, Any]] = {}
_video_probe_lock = threading.Lock()


def _probe_video_format(video: Path) -> dict[str, Any]:
    """
    Frame rate / dimensions for the UI player, cached per path+mtime+size.

    Prefers ffprobe's exact rational (24000/1001) over OpenCV's rounded
    float — frame stepping and HH:MM:SS:FF drift badly at 24 vs 23.976.
    """
    try:
        st = video.stat()
        key = (str(video), int(st.st_mtime), int(st.st_size))
    except OSError:
        return {"fps": None, "fps_num": None, "fps_den": None,
                "width": 0, "height": 0, "frame_count": 0}

    with _video_probe_lock:
        hit = _video_probe_cache.get(key)
    if hit is not None:
        return hit

    out: dict[str, Any] = {
        "fps": None, "fps_num": None, "fps_den": None,
        "width": 0, "height": 0, "frame_count": 0,
    }
    try:
        import json as _json

        from autoscene.video_proxy import _ensure_ffprobe

        proc = subprocess.run(
            [
                _ensure_ffprobe(), "-v", "quiet", "-print_format", "json",
                "-select_streams", "v:0", "-show_streams", str(video),
            ],
            check=False, capture_output=True, text=True,
        )
        stream = ((_json.loads(proc.stdout or "{}").get("streams") or [None]) or [None])[0]
        if stream:
            out["width"] = int(stream.get("width") or 0)
            out["height"] = int(stream.get("height") or 0)
            out["frame_count"] = int(stream.get("nb_frames") or 0)
            rate = stream.get("avg_frame_rate") or stream.get("r_frame_rate")
            if rate and rate not in ("N/A", "0/0"):
                text = str(rate)
                if "/" in text:
                    num, den = text.split("/", 1)
                    n, d = int(float(num)), int(float(den))
                    if d > 0 and n > 0:
                        out["fps_num"], out["fps_den"] = n, d
                        out["fps"] = n / d
                else:
                    val = float(text)
                    if val > 0:
                        out["fps"] = val
    except Exception:  # noqa: BLE001 - probing is best-effort
        log.debug("ffprobe video-meta failed for %s", video, exc_info=True)

    if not out["fps"]:
        # OpenCV fallback: rounded, but better than hiding frame controls.
        try:
            from autoscene.decode import probe_video

            info = probe_video(video)
            if info.fps > 0:
                out["fps"] = float(info.fps)
            out["width"] = out["width"] or int(info.width)
            out["height"] = out["height"] or int(info.height)
            out["frame_count"] = out["frame_count"] or int(info.frame_count)
        except Exception:  # noqa: BLE001
            log.debug("opencv video-meta fallback failed for %s", video, exc_info=True)

    with _video_probe_lock:
        _video_probe_cache[key] = out
    return out


@app.get("/api/projects/{project_id}/video-meta")
def project_video_meta(project_id: str) -> dict[str, Any]:
    """Filesystem size/path plus frame rate for the UI player."""
    try:
        project = store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    video = Path(project.video_path)
    try:
        size = video.stat().st_size if video.exists() else 0
    except OSError:
        size = 0
    meta: dict[str, Any] = {
        "video_path": str(video),
        "video_bytes": size,
        "exists": video.exists(),
    }
    if video.exists():
        meta.update(_probe_video_format(video))
    else:
        meta.update(
            {"fps": None, "fps_num": None, "fps_den": None,
             "width": 0, "height": 0, "frame_count": 0}
        )
    return meta


@app.post("/api/system/open-folder")
def open_folder(body: OpenFolderBody) -> dict[str, bool]:
    import subprocess
    import sys

    try:
        folder = assert_openable_folder(Path(body.path))
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    if not folder.exists():
        raise HTTPException(404, "Path not found")
    if sys.platform == "win32":
        subprocess.Popen(["explorer", str(folder)])  # noqa: S603
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(folder)])  # noqa: S603
    else:
        subprocess.Popen(["xdg-open", str(folder)])  # noqa: S603
    return {"ok": True}


@app.get("/api/projects/{project_id}/media/{path:path}")
def project_media(project_id: str, path: str) -> FileResponse:
    try:
        assert_project_id(project_id)
        store.load(project_id)
        base = store._dir(project_id).resolve()
        target = assert_under(base / path, base)
    except SecurityError as exc:
        raise HTTPException(400, str(exc)) from exc
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    if not target.exists():
        raise HTTPException(404, "Missing file")
    return FileResponse(target)


@app.get("/api/projects/{project_id}/live-face")
def live_face(project_id: str) -> Response:
    """Latest face crop written during detect (Import tracker hero)."""
    try:
        store.load(project_id)
    except FileNotFoundError as exc:
        raise HTTPException(404, "Project not found") from exc
    path = store.preview_latest_path(project_id)
    if not path.exists():
        raise HTTPException(404, "No live face yet")
    # Snapshot bytes — FileResponse Content-Length races with scan overwrites.
    try:
        data = path.read_bytes()
    except OSError as exc:
        raise HTTPException(404, "No live face yet") from exc
    if not data:
        raise HTTPException(404, "No live face yet")
    return Response(
        content=data,
        media_type="image/jpeg",
        headers={"Cache-Control": "no-store, max-age=0"},
    )


def _scan_job(project_id: str) -> None:
    lock = _scan_locks.setdefault(project_id, threading.Lock())
    if not lock.acquire(blocking=False):
        return
    cancel = _cancel_event(project_id)
    pause = _pause_event(project_id)
    cancel.clear()
    pause.clear()
    try:
        # Lazy imports inside try so failures mark project error (not silent freeze).
        from autoscene.pipeline import ScanCancelled, run_scan, run_scene_cuts_phase

        if store.is_deleted(project_id):
            log.info("Scan aborted; project already deleted project=%s", project_id)
            return
        project = store.load(project_id)

        def _cancelled() -> bool:
            return cancel.is_set() or store.is_deleted(project_id)

        # Phase 0: scene cuts before any face work
        run_scene_cuts_phase(
            store,
            project,
            should_cancel=_cancelled,
            force=True,
        )
        if _cancelled():
            raise ScanCancelled(project_id)

        project = store.load(project_id)
        project.status = "scanning"
        project.progress = 0.0
        project.stage = "Starting face scan…"
        project.progress_phase = "probe"
        store.save(project)

        run_scan(
            store,
            project,
            should_cancel=_cancelled,
            should_pause=lambda: pause.is_set(),
        )
    except Exception as exc:  # noqa: BLE001
        # ScanCancelled may be unbound if pipeline import failed — match by name.
        if type(exc).__name__ == "ScanCancelled":
            log.info("Scan cancelled project=%s", project_id)
            if not store.is_deleted(project_id):
                try:
                    project = store.load(project_id)
                    if project.status != "ready":
                        project.status = "idle"
                        project.stage = "Stopped"
                        project.eta_sec = None
                        store.save(project)
                except FileNotFoundError:
                    pass
            return
        if isinstance(exc, FileNotFoundError):
            log.info("Scan aborted; project gone project=%s", project_id)
            return
        log.exception("Scan failed")
        if store.is_deleted(project_id):
            return
        try:
            project = store.load(project_id)
            project.status = "error"
            project.error = str(exc)
            project.stage = "Error"
            store.save(project)
        except FileNotFoundError:
            pass
        except Exception:  # noqa: BLE001
            pass
    finally:
        lock.release()
        cancel.clear()
        pause.clear()
        if store.is_deleted(project_id):
            _scan_locks.pop(project_id, None)
            _scan_cancel.pop(project_id, None)
            _scan_pause.pop(project_id, None)


def mount_ui(dist: Path) -> None:
    if dist.exists():
        app.mount("/", StaticFiles(directory=str(dist), html=True), name="ui")


def create_app(serve_ui: bool = True) -> FastAPI:
    if serve_ui:
        root = Path(__file__).resolve().parents[2]
        mount_ui(root / "ui" / "dist")
    return app
