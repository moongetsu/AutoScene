from __future__ import annotations

import json
import logging
import os
import threading
import time
import uuid
from dataclasses import asdict, dataclass, field, fields
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from autoscene.config import settings

log = logging.getLogger(__name__)

_tmp_seq = 0
_tmp_seq_lock = threading.Lock()


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _read_json_object(path: Path) -> dict[str, Any]:
    """
    Load a JSON object from disk. Tolerates trailing garbage from a torn
    concurrent write (json.loads Extra data) by taking the first value.
    """
    text = path.read_text(encoding="utf-8-sig")
    if not text.strip():
        raise ValueError(f"Empty project meta: {path}")
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        try:
            data, end = json.JSONDecoder().raw_decode(text)
        except json.JSONDecodeError:
            raise exc from None
        trailing = text[end:].strip()
        if trailing:
            log.warning(
                "Recovered project.json with trailing garbage path=%s bytes=%s",
                path,
                len(trailing),
            )
            # Rewrite clean so the next list/load is cheap.
            try:
                _atomic_write_json(path, data)
            except OSError as write_exc:
                log.warning("Could not rewrite recovered project.json: %s", write_exc)
    if not isinstance(data, dict):
        raise ValueError(f"Project meta must be an object: {path}")
    return data


def _is_windows_lock_error(exc: BaseException) -> bool:
    if isinstance(exc, PermissionError):
        return True
    win = getattr(exc, "winerror", None)
    if win in (5, 32):  # ACCESS_DENIED, SHARING_VIOLATION
        return True
    err = getattr(exc, "errno", None)
    return err in (13, 11)  # EACCES / EAGAIN


def _write_text_fsync(path: Path, payload: str) -> None:
    """Overwrite path with fsync; used when os.replace is denied."""
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(payload)
        fh.flush()
        os.fsync(fh.fileno())


def _atomic_write_json(path: Path, data: dict[str, Any]) -> None:
    """
    Write JSON via temp + os.replace.

    On Windows, replace/overwrite can raise WinError 5/32 when Defender or
    another handle briefly holds project.json. Retry hard, then overwrite,
    then copyfile — never leave a successful in-memory scan unpersistable
    without trying every durable path.
    """
    global _tmp_seq
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(data, indent=2)
    with _tmp_seq_lock:
        _tmp_seq += 1
        seq = _tmp_seq
    tmp = path.with_name(
        f"{path.name}.{os.getpid()}.{threading.get_ident()}.{seq}.tmp"
    )
    last_exc: BaseException | None = None
    try:
        _write_text_fsync(tmp, payload)
        for attempt in range(16):
            try:
                os.replace(str(tmp), str(path))
                return
            except OSError as exc:
                if not _is_windows_lock_error(exc):
                    raise
                last_exc = exc
                time.sleep(0.04 * (attempt + 1))
        # replace still denied — try in-place overwrite
        for attempt in range(8):
            try:
                _write_text_fsync(path, payload)
                return
            except OSError as exc:
                if not _is_windows_lock_error(exc):
                    raise
                last_exc = exc
                time.sleep(0.05 * (attempt + 1))
        # Last resort: sibling file + copy
        import shutil

        sibling = path.with_suffix(".json.pending")
        _write_text_fsync(sibling, payload)
        for attempt in range(8):
            try:
                shutil.copyfile(str(sibling), str(path))
                try:
                    sibling.unlink(missing_ok=True)
                except OSError:
                    pass
                return
            except OSError as exc:
                if not _is_windows_lock_error(exc):
                    raise
                last_exc = exc
                time.sleep(0.05 * (attempt + 1))
        assert last_exc is not None
        raise last_exc
    finally:
        try:
            tmp.unlink(missing_ok=True)
        except OSError:
            pass


@dataclass
class PersonRecord:
    person_id: int
    label: str
    detection_count: int
    clip_count: int
    screen_time_sec: float
    first_sec: float
    last_sec: float
    thumbnail_path: str | None = None
    # Unit embedding centroid for teach-cast / sticky identity (optional on old projects).
    centroid: list[float] | None = None


@dataclass
class ShotRecord:
    start_sec: float
    end_sec: float


@dataclass
class SegmentRecord:
    person_id: int
    start_sec: float
    end_sec: float
    # Face-presence window inside the shot (WITH). Export uses start/end.
    preview_start_sec: float | None = None
    preview_end_sec: float | None = None
    # Timestamp of a real detection of this person inside the shot
    # (thumbnail anchor). None on legacy projects → UI falls back.
    face_time_sec: float | None = None


def trim_segments_to_face_windows(
    segments: list[SegmentRecord],
    exclusions: list[SegmentExclusion] | None = None,
) -> tuple[list[SegmentRecord], list[SegmentExclusion], bool]:
    """
    No-op for commercial full-shot packs.

    Historically this collapsed start/end onto preview_* (face windows).
    Packs now export the whole scene-cut plate; face presence stays in
    preview_* for WITH only. Kept as a stable import for older callers.
    """
    return list(segments), list(exclusions or []), False


@dataclass
class SegmentExclusion:
    person_id: int
    start_sec: float
    end_sec: float


@dataclass
class SceneRecord:
    scene_id: int
    start_sec: float
    end_sec: float
    person_ids: list[int]


@dataclass
class MergedExportRecord:
    """Completed merged export for Export landing / history."""

    id: str
    person_id: int
    person_label: str
    output_path: str
    scene_count: int
    finished_at: str
    export_mode: str = ""
    video_codec: str = ""


def segment_key(person_id: int, start_sec: float, end_sec: float) -> tuple[int, float, float]:
    return (person_id, round(start_sec, 4), round(end_sec, 4))


def is_segment_excluded(
    exclusions: list[SegmentExclusion],
    person_id: int,
    start_sec: float,
    end_sec: float,
) -> bool:
    key = segment_key(person_id, start_sec, end_sec)
    return any(
        segment_key(ex.person_id, ex.start_sec, ex.end_sec) == key for ex in exclusions
    )


@dataclass
class ProjectState:
    id: str
    name: str
    video_path: str
    created_at: str
    updated_at: str
    status: str = "idle"
    progress: float = 0.0
    stage: str = ""
    error: str | None = None
    duration_sec: float = 0.0
    # Fine-grained scan progress (frame-accurate when frames_total is known)
    progress_phase: str = ""
    frames_done: int = 0
    frames_total: int = 0
    frames_approx: bool = False
    detect_hits: int = 0
    eta_sec: float | None = None
    backend: str = "insightface"
    model: str = "buffalo_l"
    # Shot spans from pre-scan scene cut detect (full-shot export plates).
    shots: list[ShotRecord] = field(default_factory=list)
    scene_cut_detector_used: str = ""
    # Live cut-phase telemetry (provisional during TransNet).
    cuts_found: int = 0
    people: list[PersonRecord] = field(default_factory=list)
    segments: list[SegmentRecord] = field(default_factory=list)
    excluded_segments: list[SegmentExclusion] = field(default_factory=list)
    merged_exports: list[MergedExportRecord] = field(default_factory=list)
    selected_person_id: int | None = None
    # Soft-era / stale identity signal (set on load, cleared after rescan).
    needs_rescan: bool = False
    identity_pipeline_version: int = 0

    def shared_scenes(self) -> list[SceneRecord]:
        """Derive scenes from pairwise segment overlap (connected components)."""
        from autoscene.cluster import Segment, build_shared_scenes
        from autoscene.config import settings
        from autoscene.identity_gates import gate_config_for

        cfg = gate_config_for(settings.content_preset, self.backend)
        built = build_shared_scenes(
            [
                Segment(
                    s.person_id,
                    s.start_sec,
                    s.end_sec,
                    preview_start_sec=s.preview_start_sec,
                    preview_end_sec=s.preview_end_sec,
                    face_time_sec=s.face_time_sec,
                )
                for s in self.segments
            ],
            gap_sec=0.35,
            min_segment_sec=settings.min_segment_sec,
            min_presence_sec=float(cfg.cocast_min_overlap_sec),
        )
        return [
            SceneRecord(
                scene_id=s.scene_id,
                start_sec=s.start_sec,
                end_sec=s.end_sec,
                person_ids=list(s.person_ids),
            )
            for s in built
        ]

    def cocast_edges(self) -> list[dict[str, float | int]]:
        """Honest co-cast pairs for WITH UI (segment overlap, not scene union)."""
        from autoscene.cluster import Segment, cocast_pairs
        from autoscene.config import settings
        from autoscene.identity_gates import gate_config_for

        cfg = gate_config_for(settings.content_preset, self.backend)
        pairs = cocast_pairs(
            [
                Segment(
                    s.person_id,
                    s.start_sec,
                    s.end_sec,
                    preview_start_sec=s.preview_start_sec,
                    preview_end_sec=s.preview_end_sec,
                    face_time_sec=s.face_time_sec,
                )
                for s in self.segments
            ],
            min_overlap_sec=float(cfg.cocast_min_overlap_sec),
        )
        return [
            {"a": int(a), "b": int(b), "overlap_sec": round(float(sec), 4)}
            for a, b, sec in pairs
        ]

    def to_dict(self, *, light: bool | None = None) -> dict[str, Any]:
        """Full state for disk persistence (includes teach centroids)."""
        data = asdict(self)
        try:
            data["video_bytes"] = Path(self.video_path).stat().st_size
        except OSError:
            data["video_bytes"] = 0
        # During scan, skip expensive shared-scene / cocast rebuilds every tick.
        if light is None:
            light = self.status in ("scanning", "cutting", "paused", "queued")
        if light:
            data["scenes"] = []
            data["cocast_edges"] = []
        else:
            data["scenes"] = [asdict(s) for s in self.shared_scenes()]
            data["cocast_edges"] = self.cocast_edges()
        return data

    def to_api_dict(self) -> dict[str, Any]:
        """UI payload — drop fat embedding centroids (hundreds of KB per poll)."""
        data = self.to_dict()
        for person in data.get("people") or []:
            if isinstance(person, dict):
                person.pop("centroid", None)
        data["shot_count"] = len(self.shots)
        data["needs_rescan"] = bool(self.needs_rescan)
        return data

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> ProjectState:
        people = [PersonRecord(**p) for p in data.get("people", [])]
        seg_allowed = {f.name for f in fields(SegmentRecord)}
        segments = [
            SegmentRecord(**{k: v for k, v in s.items() if k in seg_allowed})
            for s in data.get("segments", [])
            if isinstance(s, dict)
        ]
        shot_allowed = {f.name for f in fields(ShotRecord)}
        shots = [
            ShotRecord(**{k: v for k, v in s.items() if k in shot_allowed})
            for s in data.get("shots", [])
            if isinstance(s, dict)
        ]
        excluded = [SegmentExclusion(**s) for s in data.get("excluded_segments", [])]
        export_allowed = {f.name for f in fields(MergedExportRecord)}
        merged_exports = [
            MergedExportRecord(**{k: v for k, v in e.items() if k in export_allowed})
            for e in data.get("merged_exports", [])
            if isinstance(e, dict)
        ]
        raw = {
            **data,
            "people": people,
            "segments": segments,
            "shots": shots,
            "excluded_segments": excluded,
            "merged_exports": merged_exports,
        }
        allowed = {f.name for f in fields(cls)}
        state = cls(**{k: v for k, v in raw.items() if k in allowed})
        state.needs_rescan = detect_needs_rescan(state)
        return state


def detect_needs_rescan(state: ProjectState) -> bool:
    """
    True when identity data predates the current pipeline (soft assign era).
    Does not delete cast; signals UI/API that a rescan is recommended.
    """
    from autoscene.identity_gates import IDENTITY_PIPELINE_VERSION

    if int(getattr(state, "identity_pipeline_version", 0) or 0) >= IDENTITY_PIPELINE_VERSION:
        return False

    if state.segments:
        with_face = sum(
            1 for s in state.segments if s.face_time_sec is not None
        )
        if with_face < len(state.segments) * 0.5:
            return True

    if state.people:
        with_cent = sum(1 for p in state.people if p.centroid)
        if with_cent < len(state.people) * 0.5:
            return True

    return True


class ProjectStore:
    def __init__(self, root: Path | None = None) -> None:
        settings.ensure_dirs()
        self.root = root or settings.projects_dir or (settings.data_dir / "projects")
        self.root.mkdir(parents=True, exist_ok=True)
        # In-memory tombstones so a still-running scan cannot recreate a deleted project
        self._deleted: set[str] = set()
        self._io_locks: dict[str, threading.RLock] = {}
        self._io_locks_guard = threading.Lock()
        # Live objects: UI polls hit memory, not project.json, while scan writes.
        self._live: dict[str, ProjectState] = {}

    def is_deleted(self, project_id: str) -> bool:
        return project_id in self._deleted

    def _io_lock(self, project_id: str) -> threading.RLock:
        with self._io_locks_guard:
            lock = self._io_locks.get(project_id)
            if lock is None:
                lock = threading.RLock()
                self._io_locks[project_id] = lock
            return lock

    def _dir(self, project_id: str) -> Path:
        from autoscene.security import assert_project_id

        return self.root / assert_project_id(project_id)

    def _meta(self, project_id: str) -> Path:
        return self._dir(project_id) / "project.json"

    def list_projects(self) -> list[ProjectState]:
        items: list[ProjectState] = []
        seen: set[str] = set()
        # Prefer live (scanning) projects first so progress stays fresh.
        for project_id, state in list(self._live.items()):
            if project_id in self._deleted:
                continue
            items.append(state)
            seen.add(project_id)
        for path in sorted(self.root.glob("*/project.json"), reverse=True):
            project_id = path.parent.name
            if project_id in self._deleted or project_id in seen:
                continue
            try:
                items.append(self.load(project_id))
            except Exception as exc:  # noqa: BLE001
                # Silent skip made the dashboard show "0 projects" while the
                # folder still existed (corrupt project.json / Extra data).
                log.exception(
                    "Skipping unreadable project id=%s path=%s err=%s",
                    project_id,
                    path,
                    exc,
                )
                continue
        return items

    def create(self, name: str, video_path: Path, backend: str, model: str) -> ProjectState:
        project_id = uuid.uuid4().hex[:12]
        self._deleted.discard(project_id)
        now = _utc_now()
        state = ProjectState(
            id=project_id,
            name=name,
            video_path=str(video_path.resolve()),
            created_at=now,
            updated_at=now,
            backend=backend,
            model=model,
        )
        d = self._dir(project_id)
        d.mkdir(parents=True, exist_ok=True)
        (d / "thumbs").mkdir(exist_ok=True)
        (d / "exports").mkdir(exist_ok=True)
        self.save(state)
        return state

    def load(self, project_id: str) -> ProjectState:
        if project_id in self._deleted:
            raise FileNotFoundError(project_id)
        with self._io_lock(project_id):
            live = self._live.get(project_id)
            if live is not None:
                return live
            meta = self._meta(project_id)
            if not meta.exists():
                raise FileNotFoundError(project_id)
            data = _read_json_object(meta)
            state = ProjectState.from_dict(data)
            segs, excl, changed = trim_segments_to_face_windows(
                state.segments, state.excluded_segments
            )
            if changed:
                state.segments = segs
                state.excluded_segments = excl
                # Refresh clip counts / screen time from trimmed windows.
                by_pid: dict[int, list[SegmentRecord]] = {}
                for s in segs:
                    by_pid.setdefault(int(s.person_id), []).append(s)
                for person in state.people:
                    own = by_pid.get(int(person.person_id), [])
                    person.clip_count = len(own)
                    person.screen_time_sec = sum(
                        max(0.0, float(s.end_sec) - float(s.start_sec)) for s in own
                    )
                    if own:
                        person.first_sec = float(own[0].start_sec)
                        person.last_sec = float(own[-1].end_sec)
                self._save_unlocked(state)
                log.info(
                    "Segment normalize project=%s clips=%s",
                    project_id,
                    len(segs),
                )
            state.needs_rescan = detect_needs_rescan(state)
            # Stuck after a crashed export + corrupt save: no queue means ready.
            if state.status == "exporting":
                try:
                    from autoscene.export_queue import export_queue

                    snap = export_queue.snapshot()
                    cur = snap.get("current") or {}
                    pending = snap.get("pending") or []
                    busy = (
                        cur.get("project_id") == state.id
                        or any(j.get("project_id") == state.id for j in pending)
                    )
                    if not busy:
                        state.status = "ready"
                        state.progress = 1.0
                        state.stage = ""
                        self._save_unlocked(state)
                except Exception:  # noqa: BLE001
                    pass
            self._live[project_id] = state
            return state

    def _save_unlocked(self, state: ProjectState, *, soft: bool = True) -> None:
        if state.id in self._deleted:
            log.info("Skip save for deleted project %s", state.id)
            return
        state.updated_at = _utc_now()
        # Memory first — UI polls must not open project.json under the writer.
        self._live[state.id] = state
        try:
            _atomic_write_json(self._meta(state.id), state.to_dict())
        except OSError as exc:
            if soft and _is_windows_lock_error(exc):
                # Scan/UI must not die on a brief AV / sharing lock.
                log.warning(
                    "save deferred (file busy) project=%s err=%s",
                    state.id,
                    exc,
                )
                return
            raise

    def save(self, state: ProjectState, *, soft: bool = True) -> None:
        with self._io_lock(state.id):
            self._save_unlocked(state, soft=soft)

    def remember(self, state: ProjectState) -> None:
        """Publish live state for UI polls without touching disk."""
        if state.id in self._deleted:
            return
        with self._io_lock(state.id):
            self._live[state.id] = state

    def thumbs_dir(self, project_id: str) -> Path:
        if project_id in self._deleted:
            raise FileNotFoundError(project_id)
        p = self._dir(project_id) / "thumbs"
        p.mkdir(exist_ok=True)
        return p

    def preview_latest_path(self, project_id: str) -> Path:
        """Single latest face crop shown live in the Import tracker."""
        if project_id in self._deleted:
            raise FileNotFoundError(project_id)
        p = self._dir(project_id) / "previews"
        p.mkdir(exist_ok=True)
        return p / "latest.jpg"

    def exports_dir(self, project_id: str) -> Path:
        if project_id in self._deleted:
            raise FileNotFoundError(project_id)
        p = self._dir(project_id) / "exports"
        p.mkdir(exist_ok=True)
        return p

    def delete(self, project_id: str) -> None:
        import shutil

        self._deleted.add(project_id)
        self._live.pop(project_id, None)
        target = self._dir(project_id).resolve()
        root = self.root.resolve()
        if target == root or root not in target.parents:
            raise ValueError(f"Refusing to delete outside projects root: {target}")
        if target.exists():
            shutil.rmtree(target, ignore_errors=False)
        # If already gone, still treat as success (idempotent delete)
