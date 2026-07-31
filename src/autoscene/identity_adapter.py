"""On-device identity adapter: linear map over face embeddings (all-in teach)."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

import numpy as np

log = logging.getLogger(__name__)


def _unit(v: np.ndarray) -> np.ndarray:
    v = np.asarray(v, dtype=np.float64).reshape(-1)
    n = float(np.linalg.norm(v))
    if n <= 1e-12:
        return v
    return v / n


class IdentityAdapter:
    """
    Learns W (d×d) so mapped embeddings better separate taught identities.
    Persisted as identity_adapter.npz under the project dir.
    """

    def __init__(self, dim: int) -> None:
        self.dim = int(dim)
        self.W = np.eye(self.dim, dtype=np.float64)

    def apply(self, emb: np.ndarray) -> np.ndarray:
        x = _unit(emb)
        if x.size != self.dim:
            return x
        return _unit(self.W @ x)

    def train(
        self,
        positives: dict[int, list[np.ndarray]],
        *,
        steps: int = 80,
        lr: float = 0.08,
    ) -> dict[str, Any]:
        """
        Contrastive push: each positive closer to own class mean than others.
        All-in for local teach without full InsightFace backbone SGD.
        """
        classes = {
            pid: [_unit(e) for e in embs if np.asarray(e).size == self.dim]
            for pid, embs in positives.items()
        }
        classes = {pid: embs for pid, embs in classes.items() if embs}
        if len(classes) < 1:
            return {"trained": False, "reason": "no positives"}

        means = {pid: _unit(np.mean(np.stack(embs), axis=0)) for pid, embs in classes.items()}
        pids = list(classes.keys())

        W = self.W.copy()
        for _ in range(max(1, steps)):
            grad = np.zeros_like(W)
            n = 0
            for pid in pids:
                for x in classes[pid]:
                    mx = _unit(W @ x)
                    # Attract to own mean in mapped space
                    own = means[pid]
                    # Push away from nearest other mean
                    others = [means[o] for o in pids if o != pid]
                    if others:
                        sims = [float(np.dot(mx, o)) for o in others]
                        j = int(np.argmax(sims))
                        neg = others[j]
                        # d/dW of (own·Wx - neg·Wx) via outer products
                        grad += np.outer(own - neg, x)
                        n += 1
                    else:
                        grad += np.outer(own, x)
                        n += 1
            if n:
                W = W + lr * (grad / n)
                # Mild orthonormalize toward stable map
                u, _, vt = np.linalg.svd(W, full_matrices=False)
                W = 0.85 * W + 0.15 * (u @ vt)

        self.W = W
        return {"trained": True, "classes": len(classes), "steps": steps}

    def training_collapsed_distinct_people(
        self,
        positives: dict[int, list[np.ndarray]],
        assign_min_sim: float,
        *,
        slack: float = 0.03,
        hard_ceiling: float = 0.90,
    ) -> tuple[bool, float]:
        """
        After train, check mapped class means did not collapse distinct people.
        Returns (collapsed, max_pairwise_sim).
        """
        classes = {
            pid: [_unit(e) for e in embs if np.asarray(e).size == self.dim]
            for pid, embs in positives.items()
        }
        classes = {pid: embs for pid, embs in classes.items() if embs}
        pids = list(classes.keys())
        if len(pids) < 2:
            return False, 0.0

        means: dict[int, np.ndarray] = {}
        for pid, embs in classes.items():
            mapped = [_unit(self.W @ x) for x in embs]
            means[pid] = _unit(np.mean(np.stack(mapped), axis=0))

        max_sim = -1.0
        for i, a in enumerate(pids):
            for b in pids[i + 1 :]:
                sim = float(np.dot(means[a], means[b]))
                max_sim = max(max_sim, sim)

        threshold = max(float(assign_min_sim) - float(slack), 0.0)
        collapsed = max_sim >= threshold or max_sim >= float(hard_ceiling)
        return collapsed, max_sim

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(str(path), W=self.W, dim=np.asarray([self.dim]))

    @classmethod
    def load(cls, path: Path) -> IdentityAdapter | None:
        if not path.is_file():
            return None
        try:
            data = np.load(str(path))
            dim = int(data["dim"][0]) if "dim" in data else int(data["W"].shape[0])
            ad = cls(dim)
            ad.W = np.asarray(data["W"], dtype=np.float64)
            return ad
        except Exception as exc:  # noqa: BLE001
            log.warning("identity adapter load failed: %s", exc)
            return None


def adapter_path(project_dir: Path) -> Path:
    return project_dir / "identity_adapter.npz"


def detection_cache_path(project_dir: Path) -> Path:
    return project_dir / "detection_cache.npz"


def save_detection_cache(
    project_dir: Path,
    *,
    timestamps: np.ndarray,
    embeddings: np.ndarray,
    labels: np.ndarray,
    bboxes: np.ndarray | None = None,
    det_scores: np.ndarray | None = None,
) -> None:
    path = detection_cache_path(project_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload: dict[str, np.ndarray] = {
        "timestamps": np.asarray(timestamps, dtype=np.float64),
        "embeddings": np.asarray(embeddings, dtype=np.float32),
        "labels": np.asarray(labels, dtype=np.int32),
    }
    # Geometry enables tracklet gating on teach-rebuild (new scans only).
    if bboxes is not None:
        payload["bboxes"] = np.asarray(bboxes, dtype=np.float32)
    if det_scores is not None:
        payload["det_scores"] = np.asarray(det_scores, dtype=np.float32)
    np.savez_compressed(str(path), **payload)


def load_detection_cache(project_dir: Path) -> dict[str, np.ndarray] | None:
    """Cache dict: timestamps, embeddings, labels (+bboxes/det_scores if new)."""
    path = detection_cache_path(project_dir)
    if not path.is_file():
        return None
    try:
        data = np.load(str(path))
        out: dict[str, np.ndarray] = {
            "timestamps": np.asarray(data["timestamps"], dtype=np.float64),
            "embeddings": np.asarray(data["embeddings"], dtype=np.float64),
            "labels": np.asarray(data["labels"], dtype=np.int32),
        }
        if "bboxes" in data:
            out["bboxes"] = np.asarray(data["bboxes"], dtype=np.float64)
        if "det_scores" in data:
            out["det_scores"] = np.asarray(data["det_scores"], dtype=np.float64)
        return out
    except Exception as exc:  # noqa: BLE001
        log.warning("detection cache load failed: %s", exc)
        return None


def reassign_labels_with_adapter(
    embeddings: np.ndarray,
    centroids: dict[int, np.ndarray],
    adapter: IdentityAdapter | None,
    *,
    cfg,
    timestamps: np.ndarray | None = None,
    bboxes: np.ndarray | None = None,
    det_scores: np.ndarray | None = None,
    shots: list | None = None,
) -> np.ndarray:
    """
    Gated reassignment in adapter space (identity_gates.GateConfig `cfg`).
    With geometry available the full tracklet gate applies; legacy caches
    without bboxes run the degraded per-detection gate (floor + margin +
    member purity + median confirm) — never a bare nearest-centroid.
    """
    from autoscene.identity_gates import (
        build_tracklets,
        gated_assign,
        identity_tracks,
        shot_ids_for,
    )

    n = len(embeddings)
    out = np.full(n, -1, dtype=np.int32)
    if n == 0 or not centroids:
        return out

    def _map(v: np.ndarray) -> np.ndarray:
        return adapter.apply(v) if adapter else _unit(v)

    pids = list(centroids.keys())
    C = np.stack([_map(centroids[p]) for p in pids], axis=0)
    Xm = np.stack([_map(np.asarray(embeddings[i])) for i in range(n)], axis=0)

    from autoscene.identity_gates import quality_mask

    eligible = None
    if det_scores is not None:
        eligible = quality_mask(
            np.asarray(det_scores),
            np.asarray(embeddings),
            cfg,
            bboxes=bboxes,
        )

    tracks: list[list[int]] | None = None
    if bboxes is not None and timestamps is not None:
        shot_ids = shot_ids_for(np.asarray(timestamps), shots)
        tracks = identity_tracks(
            build_tracklets(
                np.asarray(timestamps),
                np.asarray(bboxes),
                shot_ids,
                cfg,
                eligible=eligible,
            ),
            cfg,
        )

    rows = gated_assign(Xm, C, cfg, tracks=tracks, quality=det_scores)
    for i in range(n):
        if rows[i] >= 0:
            out[i] = int(pids[int(rows[i])])
    return out
