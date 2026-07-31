"""Background FIFO export queue — keeps API responsive during encode."""

from __future__ import annotations

import logging
import threading
import time
import uuid
from collections import deque
from dataclasses import asdict, dataclass, field
from typing import Any, Callable, Literal

log = logging.getLogger(__name__)

JobStatus = Literal["queued", "running", "done", "failed", "cancelled"]


@dataclass
class ExportJob:
    id: str
    project_id: str
    person_id: int
    person_label: str
    output_path: str
    export_mode: str
    video_codec: str
    reencode: bool
    segments: list[dict[str, float]]  # {start_sec, end_sec}
    scene_count: int
    status: JobStatus = "queued"
    progress: float = 0.0
    stage: str = "Queued"
    error: str | None = None
    created_at: float = field(default_factory=time.time)
    started_at: float | None = None
    finished_at: float | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class ExportQueue:
    def __init__(self) -> None:
        self._q: deque[ExportJob] = deque()
        self._lock = threading.Lock()
        self._wake = threading.Event()
        self._current: ExportJob | None = None
        self._cancel_current = threading.Event()
        self._history: list[ExportJob] = []  # recent done/failed
        self._worker = threading.Thread(
            target=self._loop, name="autoscene-export-queue", daemon=True
        )
        self._started = False
        # Injected by API after store exists
        self._run_job: Callable[[ExportJob, Callable[[], bool]], None] | None = None

    def start(self, run_job: Callable[[ExportJob, Callable[[], bool]], None]) -> None:
        self._run_job = run_job
        if not self._started:
            self._started = True
            self._worker.start()
            log.info("Export queue worker started")

    def enqueue(self, job: ExportJob) -> ExportJob:
        with self._lock:
            self._q.append(job)
            pos = len(self._q) + (1 if self._current else 0)
        self._wake.set()
        log.info(
            "Export queued job=%s project=%s person=%s pos~%s out=%s",
            job.id,
            job.project_id,
            job.person_id,
            pos,
            job.output_path,
        )
        return job

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            current = self._current.to_dict() if self._current else None
            pending = [j.to_dict() for j in self._q]
            recent = [j.to_dict() for j in self._history[-8:]]
            active = self._current is not None or bool(self._q)
        return {
            "active": active,
            "current": current,
            "pending": pending,
            "recent": recent,
            "pending_count": len(pending),
        }

    def cancel_current_and_clear(self) -> dict[str, Any]:
        """Cancel running job, drop pending queue, kill ffmpeg children."""
        cancelled_ids: list[str] = []
        with self._lock:
            while self._q:
                job = self._q.popleft()
                job.status = "cancelled"
                job.stage = "Cancelled"
                job.finished_at = time.time()
                self._history.append(job)
                cancelled_ids.append(job.id)
            if self._current:
                self._cancel_current.set()
                cancelled_ids.append(self._current.id)
        _kill_ffmpeg_export_procs()
        self._wake.set()
        log.info("Export queue cleared cancelled=%s", cancelled_ids)
        return {"ok": True, "cancelled_ids": cancelled_ids}

    def cancel_for_project(self, project_id: str) -> dict[str, Any]:
        """Cancel running/pending jobs for one project only."""
        cancelled_ids: list[str] = []
        kill = False
        with self._lock:
            kept: deque[ExportJob] = deque()
            while self._q:
                job = self._q.popleft()
                if job.project_id == project_id:
                    job.status = "cancelled"
                    job.stage = "Cancelled"
                    job.finished_at = time.time()
                    self._history.append(job)
                    cancelled_ids.append(job.id)
                else:
                    kept.append(job)
            self._q = kept
            if self._current and self._current.project_id == project_id:
                self._cancel_current.set()
                cancelled_ids.append(self._current.id)
                kill = True
        if kill:
            _kill_ffmpeg_export_procs()
        self._wake.set()
        log.info(
            "Export queue cancel project=%s cancelled=%s",
            project_id,
            cancelled_ids,
        )
        return {"ok": True, "cancelled_ids": cancelled_ids}

    def _loop(self) -> None:
        while True:
            self._wake.wait(timeout=1.0)
            self._wake.clear()
            job: ExportJob | None = None
            with self._lock:
                if self._current is None and self._q:
                    job = self._q.popleft()
                    self._current = job
                    self._cancel_current.clear()
            if job is None:
                continue
            if not self._run_job:
                job.status = "failed"
                job.error = "Export runner not configured"
                self._finish(job)
                continue

            job.status = "running"
            job.started_at = time.time()
            job.stage = "Starting export…"
            job.progress = 0.0
            try:
                self._run_job(job, self._cancel_current.is_set)
                if self._cancel_current.is_set():
                    job.status = "cancelled"
                    job.stage = "Cancelled"
                    job.error = "Cancelled by user"
                elif job.status == "running":
                    job.status = "done"
                    job.progress = 1.0
                    job.stage = "Export complete"
            except Exception as exc:  # noqa: BLE001
                if self._cancel_current.is_set():
                    job.status = "cancelled"
                    job.stage = "Cancelled"
                    job.error = "Cancelled by user"
                else:
                    job.status = "failed"
                    job.stage = "Failed"
                    job.error = str(exc)[:800]
                    log.exception("Export job failed id=%s", job.id)
            finally:
                self._finish(job)

    def _finish(self, job: ExportJob) -> None:
        job.finished_at = time.time()
        with self._lock:
            if self._current and self._current.id == job.id:
                self._current = None
            self._history.append(job)
            if len(self._history) > 24:
                self._history = self._history[-24:]
            more = bool(self._q)
        if more:
            self._wake.set()


def _kill_ffmpeg_export_procs() -> None:
    from autoscene.export import kill_active_export_ffmpeg

    killed = kill_active_export_ffmpeg()
    if killed:
        log.info("Killed %s ffmpeg export process(es)", killed)


export_queue = ExportQueue()


def new_job_id() -> str:
    return uuid.uuid4().hex[:10]
